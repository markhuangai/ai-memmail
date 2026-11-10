use std::fmt::Debug;
use std::io::{Read, Write};
use std::net::TcpStream;

use async_imap as imap;
use chrono::{DateTime, Utc};
use futures_util::io::AllowStdIo;
use imap::imap_proto::{AttributeValue, MailboxDatum, Response, Status};
use imap::types::NameAttribute;
use lettre::message::{header, Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use native_tls::TlsConnector;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};

use crate::config::{MailboxConfig, SmtpConfig};
use crate::mail::{
    accepted_condition_recipient_filter_values, accepted_conditions_can_prefilter_by_recipient,
    message_matches_accepted_conditions, parse_inbound_message, validate_composed_email,
    validate_outbound_action, ComposedEmail, InboundMessage, MailError, OutboundAction,
    SentFetchBatch, SentMessage, SentSyncCursor, ACCEPTED_CONDITION_RECIPIENT_HEADERS,
};

type ImapSession<T> = imap::Session<Compat<AllowStdIo<T>>>;

pub(crate) fn fetch_unseen_blocking(
    mailbox: &MailboxConfig,
    limit: usize,
) -> Result<Vec<InboundMessage>, MailError> {
    futures_executor::block_on(async {
        if mailbox.imap.tls {
            let session = login_imap_tls(mailbox).await?;
            fetch_unseen_with_session(session, mailbox, limit).await
        } else {
            let session = login_imap_plain(mailbox).await?;
            fetch_unseen_with_session(session, mailbox, limit).await
        }
    })
    .map_err(|error| protect_imap_error(error, mailbox))
}

pub(crate) fn fetch_sent_blocking(
    mailbox: &MailboxConfig,
    cursor: Option<&SentSyncCursor>,
    backfill_cutoff: i64,
    limit: usize,
) -> Result<SentFetchBatch, MailError> {
    futures_executor::block_on(async {
        if mailbox.imap.tls {
            let session = login_imap_tls(mailbox).await?;
            fetch_sent_with_session(session, mailbox, cursor, backfill_cutoff, limit).await
        } else {
            let session = login_imap_plain(mailbox).await?;
            fetch_sent_with_session(session, mailbox, cursor, backfill_cutoff, limit).await
        }
    })
    .map_err(|error| protect_imap_error(error, mailbox))
}

async fn fetch_sent_with_session<T: Read + Write + Unpin + Debug + Send>(
    mut session: ImapSession<T>,
    mailbox: &MailboxConfig,
    cursor: Option<&SentSyncCursor>,
    backfill_cutoff: i64,
    limit: usize,
) -> Result<SentFetchBatch, MailError> {
    let folder_name = sent_folder_name(&mut session, mailbox).await?;
    let selected = session
        .examine(&folder_name)
        .await
        .map_err(|error| imap_operation_error("EXAMINE", error))?;
    let uid_validity = selected
        .uid_validity
        .ok_or_else(|| MailError::Imap("Sent mailbox did not report UIDVALIDITY".to_string()))?
        as u64;
    let (last_uid, cutoff) = sent_sync_range(cursor, &folder_name, uid_validity, backfill_cutoff);
    let mut uids = sent_uids(&mut session, last_uid, cutoff).await?;
    uids.sort_unstable();
    let complete = uids.len() <= limit;
    uids.truncate(limit);
    let mut messages = Vec::with_capacity(uids.len());
    for uid in uids {
        let responses = imap_command_responses(
            &mut session,
            "UID FETCH",
            &format!("{uid} (UID INTERNALDATE BODY.PEEK[])"),
            |response| fetched_message(response, mailbox, uid_validity, uid),
        )
        .await?;
        for (message, internal_date) in responses {
            messages.push(SentMessage {
                message,
                internal_date,
            });
        }
    }
    if let Err(error) = session.logout().await {
        let error = protect_imap_error(imap_operation_error("LOGOUT", error), mailbox);
        tracing::warn!(%error, operation = "LOGOUT", "IMAP logout failed after completed operation");
    }
    Ok(SentFetchBatch {
        folder_name,
        uid_validity,
        messages,
        complete,
    })
}

async fn sent_folder_name<T: Read + Write + Unpin + Debug + Send>(
    session: &mut ImapSession<T>,
    mailbox: &MailboxConfig,
) -> Result<String, MailError> {
    if let Some(folder) = mailbox.imap.sent_folder.as_deref() {
        return Ok(folder.trim().to_string());
    }
    let matches = imap_command_responses(session, "LIST", "\"\" \"*\"", |response| {
        let Response::MailboxData(MailboxDatum::List(name)) = response else {
            return Ok(None);
        };
        Ok(name
            .name_attributes
            .iter()
            .any(|attribute| matches!(attribute, NameAttribute::Sent))
            .then(|| name.name.to_string()))
    })
    .await?;
    match matches.as_slice() {
        [folder] => Ok(folder.clone()),
        [] => Err(MailError::Imap(
            "IMAP server did not advertise a \\Sent mailbox; configure imap.sent_folder"
                .to_string(),
        )),
        _ => Err(MailError::Imap(
            "IMAP server advertised multiple \\Sent mailboxes; configure imap.sent_folder"
                .to_string(),
        )),
    }
}

async fn sent_uids<T: Read + Write + Unpin + Debug + Send>(
    session: &mut ImapSession<T>,
    last_uid: u64,
    cutoff: i64,
) -> Result<Vec<u32>, MailError> {
    let Some(query) = sent_uid_query(last_uid, cutoff)? else {
        return Ok(Vec::new());
    };
    uid_search(session, &query).await
}

fn sent_sync_range(
    cursor: Option<&SentSyncCursor>,
    folder_name: &str,
    uid_validity: u64,
    requested_cutoff: i64,
) -> (u64, i64) {
    match cursor
        .filter(|cursor| cursor.folder_name == folder_name && cursor.uid_validity == uid_validity)
    {
        Some(cursor) if requested_cutoff >= cursor.backfill_cutoff => {
            (cursor.last_uid, cursor.backfill_cutoff)
        }
        _ => (0, requested_cutoff),
    }
}

fn sent_uid_query(last_uid: u64, cutoff: i64) -> Result<Option<String>, MailError> {
    let cutoff = DateTime::<Utc>::from_timestamp(cutoff, 0)
        .ok_or_else(|| MailError::Imap("Sent backfill cutoff is invalid".to_string()))?;
    let since = cutoff.format("%d-%b-%Y");
    let query = if last_uid == 0 {
        format!("SINCE {since}")
    } else if last_uid >= u32::MAX as u64 {
        return Ok(None);
    } else {
        format!("UID {}:{} SINCE {since}", last_uid + 1, u32::MAX)
    };
    Ok(Some(query))
}

async fn fetch_unseen_with_session<T: Read + Write + Unpin + Debug + Send>(
    mut session: ImapSession<T>,
    mailbox: &MailboxConfig,
    limit: usize,
) -> Result<Vec<InboundMessage>, MailError> {
    let selected = session
        .select(&mailbox.imap.folder)
        .await
        .map_err(|error| imap_operation_error("SELECT", error))?;
    let uid_validity = selected.uid_validity.unwrap_or_default() as u64;
    let mut uids = search_unseen_uids(&mut session, mailbox).await?;
    uids.sort_unstable();

    let mut messages = Vec::new();
    for uid in uids {
        if messages.len() >= limit {
            break;
        }
        let responses = imap_command_responses(
            &mut session,
            "UID FETCH",
            &format!("{uid} (UID BODY.PEEK[])"),
            |response| fetched_message(response, mailbox, uid_validity, uid),
        )
        .await?;
        for (message, _) in responses {
            if message_matches_accepted_conditions(&message, &mailbox.accepted_conditions) {
                messages.push(message);
                if messages.len() >= limit {
                    break;
                }
            }
        }
    }
    if let Err(error) = session.logout().await {
        let error = protect_imap_error(imap_operation_error("LOGOUT", error), mailbox);
        tracing::warn!(%error, operation = "LOGOUT", "IMAP logout failed after completed operation");
    }
    Ok(messages)
}

async fn search_unseen_uids<T: Read + Write + Unpin + Debug + Send>(
    session: &mut ImapSession<T>,
    mailbox: &MailboxConfig,
) -> Result<Vec<u32>, MailError> {
    if let Some(queries) = recipient_prefilter_queries(mailbox) {
        let mut uids = Vec::new();
        for query in queries {
            for uid in uid_search(session, &query).await? {
                if !uids.contains(&uid) {
                    uids.push(uid);
                }
            }
        }
        return Ok(uids);
    }
    uid_search(session, "UNSEEN").await
}

fn recipient_prefilter_queries(mailbox: &MailboxConfig) -> Option<Vec<String>> {
    if !accepted_conditions_can_prefilter_by_recipient(&mailbox.accepted_conditions) {
        return None;
    }
    let recipients = accepted_condition_recipient_filter_values(&mailbox.accepted_conditions);
    if recipients.is_empty() {
        return None;
    }
    Some(
        recipients
            .into_iter()
            .flat_map(|recipient| {
                let recipient = imap_search_string(&recipient);
                ACCEPTED_CONDITION_RECIPIENT_HEADERS
                    .iter()
                    .map(move |header| format!("UNSEEN HEADER {header} {recipient}"))
            })
            .collect(),
    )
}

async fn uid_search<T: Read + Write + Unpin + Debug + Send>(
    session: &mut ImapSession<T>,
    query: &str,
) -> Result<Vec<u32>, MailError> {
    Ok(
        imap_command_responses(session, "UID SEARCH", query, |response| {
            Ok(match response {
                Response::MailboxData(MailboxDatum::Search(uids)) => Some(uids.clone()),
                _ => None,
            })
        })
        .await?
        .into_iter()
        .flatten()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect(),
    )
}

// async-imap's typed LIST/SEARCH/FETCH/STORE readers discard tagged NO/BAD responses.
async fn imap_command_responses<T: Read + Write + Unpin + Debug + Send, R>(
    session: &mut ImapSession<T>,
    operation: &str,
    arguments: &str,
    mut collect: impl FnMut(&Response<'_>) -> Result<Option<R>, MailError>,
) -> Result<Vec<R>, MailError> {
    let tag = session
        .run_command(format!("{operation} {arguments}"))
        .await
        .map_err(|error| imap_operation_error(operation, error))?;
    let mut responses = Vec::new();
    while let Some(response) = session
        .read_response()
        .await
        .map_err(|error| imap_read_error(operation, &error))?
    {
        match response.parsed() {
            Response::Done {
                tag: response_tag,
                status,
                outcome,
            } if response_tag == &tag => {
                return if *status == Status::Ok {
                    Ok(responses)
                } else {
                    Err(MailError::Imap(format!(
                        "{operation} rejected ({status:?}): {}",
                        outcome.information.as_deref().unwrap_or("no server detail")
                    )))
                };
            }
            Response::Data {
                status: Status::Bye,
                ..
            } => {
                return Err(MailError::Imap(format!(
                    "{operation}: server closed the connection"
                )))
            }
            other => {
                if let Some(value) = collect(other)? {
                    responses.push(value);
                }
            }
        }
    }
    Err(MailError::Imap(format!(
        "{operation} ended before tagged completion"
    )))
}

fn imap_read_error(operation: &str, error: &std::io::Error) -> MailError {
    // The decoder reports malformed input as Other with response bytes in its error text.
    let cause = if error.kind() == std::io::ErrorKind::Other {
        "invalid IMAP response (payload omitted)".to_string()
    } else {
        format!("{:?}", error.kind())
    };
    MailError::Imap(format!("{operation} failed: {cause}"))
}

fn imap_operation_error(operation: &str, error: imap::error::Error) -> MailError {
    match error {
        imap::error::Error::Io(error) => imap_read_error(operation, &error),
        imap::error::Error::Parse(_) => MailError::Imap(format!(
            "{operation} failed: invalid IMAP response (payload omitted)"
        )),
        other => MailError::Imap(format!("{operation} failed: {other}")),
    }
}

fn fetched_message(
    response: &Response<'_>,
    mailbox: &MailboxConfig,
    uid_validity: u64,
    requested_uid: u32,
) -> Result<Option<(InboundMessage, Option<i64>)>, MailError> {
    let Response::Fetch(_, attributes) = response else {
        return Ok(None);
    };
    let mut uid = requested_uid;
    let mut body = None;
    let mut internal_date = None;
    for attribute in attributes {
        match attribute {
            AttributeValue::Uid(value) => uid = *value,
            AttributeValue::BodySection {
                section: None,
                data: Some(data),
                ..
            }
            | AttributeValue::Rfc822(Some(data)) => body = Some(data.as_ref()),
            AttributeValue::InternalDate(value) => {
                internal_date = DateTime::parse_from_str(value, "%d-%b-%Y %H:%M:%S %z")
                    .ok()
                    .map(|date| date.timestamp())
            }
            _ => {}
        }
    }
    let body = body.ok_or_else(|| MailError::Imap(format!("message uid {uid} had no body")))?;
    Ok(Some((
        parse_inbound_message(&mailbox.id, uid_validity, uid as u64, body)?,
        internal_date,
    )))
}

fn imap_search_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub(crate) fn mark_seen_blocking(mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
    futures_executor::block_on(async {
        if mailbox.imap.tls {
            let session = login_imap_tls(mailbox).await?;
            mark_seen_with_session(session, mailbox, uid).await
        } else {
            let session = login_imap_plain(mailbox).await?;
            mark_seen_with_session(session, mailbox, uid).await
        }
    })
    .map_err(|error| protect_imap_error(error, mailbox))
}

async fn mark_seen_with_session<T: Read + Write + Unpin + Debug + Send>(
    mut session: ImapSession<T>,
    mailbox: &MailboxConfig,
    uid: u64,
) -> Result<(), MailError> {
    session
        .select(&mailbox.imap.folder)
        .await
        .map_err(|error| imap_operation_error("SELECT", error))?;
    imap_command_responses(
        &mut session,
        "UID STORE",
        &format!("{uid} +FLAGS (\\Seen)"),
        |_| Ok(None::<()>),
    )
    .await?;
    if let Err(error) = session.logout().await {
        let error = protect_imap_error(imap_operation_error("LOGOUT", error), mailbox);
        tracing::warn!(%error, operation = "LOGOUT", "IMAP logout failed after completed operation");
    }
    Ok(())
}

async fn login_imap_tls(
    mailbox: &MailboxConfig,
) -> Result<ImapSession<native_tls::TlsStream<TcpStream>>, MailError> {
    let stream = TcpStream::connect((mailbox.imap.host.as_str(), mailbox.imap.port))
        .map_err(|error| MailError::Imap(format!("CONNECT failed: {error}")))?;
    let connector = TlsConnector::builder()
        .build()
        .map_err(|error| MailError::Imap(format!("TLS configuration failed: {error}")))?;
    let stream = connector
        .connect(&mailbox.imap.host, stream)
        .map_err(|error| MailError::Imap(format!("TLS handshake failed: {error}")))?;
    login_imap_stream(stream, mailbox).await
}

async fn login_imap_plain(mailbox: &MailboxConfig) -> Result<ImapSession<TcpStream>, MailError> {
    let stream = TcpStream::connect((mailbox.imap.host.as_str(), mailbox.imap.port))
        .map_err(|error| MailError::Imap(format!("CONNECT failed: {error}")))?;
    login_imap_stream(stream, mailbox).await
}

async fn login_imap_stream<T: Read + Write + Unpin + Debug + Send>(
    stream: T,
    mailbox: &MailboxConfig,
) -> Result<ImapSession<T>, MailError> {
    let mut client = imap::Client::new(AllowStdIo::new(stream).compat());
    let greeting = client
        .read_response()
        .await
        .map_err(|error| imap_read_error("GREETING", &error))?
        .ok_or_else(|| MailError::Imap("server closed before GREETING".to_string()))?;
    if !matches!(
        greeting.parsed(),
        imap::imap_proto::Response::Data {
            status: imap::imap_proto::Status::Ok,
            ..
        }
    ) {
        return Err(MailError::Imap("server GREETING was not OK".to_string()));
    }
    client
        .login(&mailbox.imap.username, &mailbox.imap.password)
        .await
        .map_err(|(error, _)| imap_operation_error("LOGIN", error))
}

fn protect_imap_error(error: MailError, mailbox: &MailboxConfig) -> MailError {
    match error {
        MailError::Imap(detail) if !mailbox.imap.password.is_empty() => {
            MailError::Imap(detail.replace(&mailbox.imap.password, crate::config::REDACTED_SECRET))
        }
        other => other,
    }
}

pub(crate) fn send_blocking(smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
    validate_outbound_action(action).map_err(|errors| {
        MailError::Build(
            errors
                .into_iter()
                .map(|error| format!("{}: {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;

    send_built_message(smtp, build_message(smtp, action)?)
}

pub(crate) fn send_composed_blocking(
    smtp: &SmtpConfig,
    message: &ComposedEmail,
) -> Result<(), MailError> {
    validate_composed_email(message).map_err(|errors| {
        MailError::Build(
            errors
                .into_iter()
                .map(|error| format!("{}: {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;

    send_built_message(smtp, build_composed_message(smtp, message)?)
}

fn send_built_message(smtp: &SmtpConfig, message: Message) -> Result<(), MailError> {
    let credentials = Credentials::new(smtp.username.clone(), smtp.password.clone());
    let mailer = if smtp.starttls {
        SmtpTransport::starttls_relay(&smtp.host)
            .map_err(|error| MailError::Smtp(error.to_string()))?
            .port(smtp.port)
            .credentials(credentials)
            .build()
    } else {
        SmtpTransport::builder_dangerous(&smtp.host)
            .port(smtp.port)
            .credentials(credentials)
            .build()
    };
    mailer
        .send(&message)
        .map_err(|error| MailError::Smtp(error.to_string()))?;
    Ok(())
}

pub(crate) fn build_message(
    smtp: &SmtpConfig,
    action: &OutboundAction,
) -> Result<Message, MailError> {
    let mut builder = Message::builder()
        .from(parse_mailbox(&smtp.from)?)
        .subject(action.subject.clone());
    if let Some(message_id) = &action.message_id {
        builder = builder.message_id(Some(message_id.clone()));
    }
    if let Some(in_reply_to) = &action.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.clone());
    }
    if let Some(reply_to) = &action.reply_to {
        builder = builder.reply_to(parse_mailbox(reply_to)?);
    }
    if !action.references.is_empty() {
        builder = builder.references(action.references.join(" "));
    }
    for recipient in &action.recipients {
        builder = builder.to(parse_mailbox(recipient)?);
    }
    if let Some(html_body) = &action.html_body {
        builder = builder.header(header::ContentType::TEXT_HTML);
        builder
            .body(html_body.clone())
            .map_err(|error| MailError::Build(error.to_string()))
    } else {
        builder = builder.header(header::ContentType::TEXT_PLAIN);
        builder
            .body(action.body.clone())
            .map_err(|error| MailError::Build(error.to_string()))
    }
}

pub(crate) fn build_composed_message(
    smtp: &SmtpConfig,
    message: &ComposedEmail,
) -> Result<Message, MailError> {
    let mut builder = Message::builder()
        .from(parse_mailbox(&smtp.from)?)
        .subject(message.subject.clone());
    if let Some(message_id) = &message.message_id {
        builder = builder.message_id(Some(message_id.clone()));
    }
    if let Some(in_reply_to) = &message.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.clone());
    }
    if !message.references.is_empty() {
        builder = builder.references(message.references.join(" "));
    }
    for recipient in &message.to {
        builder = builder.to(parse_mailbox(recipient)?);
    }
    for recipient in &message.cc {
        builder = builder.cc(parse_mailbox(recipient)?);
    }
    for recipient in &message.bcc {
        builder = builder.bcc(parse_mailbox(recipient)?);
    }
    if let Some(html_body) = &message.html_body {
        builder
            .multipart(MultiPart::alternative_plain_html(
                message.text_body.clone(),
                html_body.clone(),
            ))
            .map_err(|error| MailError::Build(error.to_string()))
    } else {
        builder = builder.header(header::ContentType::TEXT_PLAIN);
        builder
            .body(message.text_body.clone())
            .map_err(|error| MailError::Build(error.to_string()))
    }
}

pub(crate) fn parse_mailbox(value: &str) -> Result<Mailbox, MailError> {
    value
        .parse::<Mailbox>()
        .map_err(|error| MailError::Build(error.to_string()))
}

#[cfg(test)]
pub(crate) mod protocol_tests;
#[cfg(test)]
mod tests;
