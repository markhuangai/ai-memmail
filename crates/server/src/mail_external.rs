use std::io::{Read, Write};
use std::net::TcpStream;

use chrono::{DateTime, Utc};
use imap::types::NameAttribute;
use lettre::message::{header, Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use native_tls::TlsConnector;

use crate::config::{MailboxConfig, SmtpConfig};
use crate::mail::{
    accepted_condition_recipient_filter_values, accepted_conditions_can_prefilter_by_recipient,
    message_matches_accepted_conditions, parse_inbound_message, validate_composed_email,
    validate_outbound_action, ComposedEmail, InboundMessage, MailError, OutboundAction,
    SentFetchBatch, SentMessage, SentSyncCursor, ACCEPTED_CONDITION_RECIPIENT_HEADERS,
};

pub(crate) fn fetch_unseen_blocking(
    mailbox: &MailboxConfig,
    limit: usize,
) -> Result<Vec<InboundMessage>, MailError> {
    if mailbox.imap.tls {
        let session = login_imap_tls(mailbox)?;
        fetch_unseen_with_session(session, mailbox, limit)
    } else {
        let session = login_imap_plain(mailbox)?;
        fetch_unseen_with_session(session, mailbox, limit)
    }
}

pub(crate) fn fetch_sent_blocking(
    mailbox: &MailboxConfig,
    cursor: Option<&SentSyncCursor>,
    backfill_cutoff: i64,
    limit: usize,
) -> Result<SentFetchBatch, MailError> {
    if mailbox.imap.tls {
        let session = login_imap_tls(mailbox)?;
        fetch_sent_with_session(session, mailbox, cursor, backfill_cutoff, limit)
    } else {
        let session = login_imap_plain(mailbox)?;
        fetch_sent_with_session(session, mailbox, cursor, backfill_cutoff, limit)
    }
}

fn fetch_sent_with_session<T: Read + Write>(
    mut session: imap::Session<T>,
    mailbox: &MailboxConfig,
    cursor: Option<&SentSyncCursor>,
    backfill_cutoff: i64,
    limit: usize,
) -> Result<SentFetchBatch, MailError> {
    let folder_name = sent_folder_name(&mut session, mailbox)?;
    let selected = session
        .examine(&folder_name)
        .map_err(|error| MailError::Imap(error.to_string()))?;
    let uid_validity = selected
        .uid_validity
        .ok_or_else(|| MailError::Imap("Sent mailbox did not report UIDVALIDITY".to_string()))?
        as u64;
    let (last_uid, cutoff) = sent_sync_range(cursor, &folder_name, uid_validity, backfill_cutoff);
    let mut uids = sent_uids(&mut session, last_uid, cutoff)?;
    uids.sort_unstable();
    let complete = uids.len() <= limit;
    uids.truncate(limit);
    let mut messages = Vec::with_capacity(uids.len());
    for uid in uids {
        let fetches = session
            .uid_fetch(uid.to_string(), "(UID INTERNALDATE BODY.PEEK[])")
            .map_err(|error| MailError::Imap(error.to_string()))?;
        for fetch in fetches.iter() {
            let raw = fetch
                .body()
                .ok_or_else(|| MailError::Imap(format!("sent message uid {uid} had no body")))?;
            let fetched_uid = fetch.uid.unwrap_or(uid) as u64;
            messages.push(SentMessage {
                message: parse_inbound_message(&mailbox.id, uid_validity, fetched_uid, raw)?,
                internal_date: fetch.internal_date().map(|date| date.timestamp()),
            });
        }
    }
    let _ = session.logout();
    Ok(SentFetchBatch {
        folder_name,
        uid_validity,
        messages,
        complete,
    })
}

fn sent_folder_name<T: Read + Write>(
    session: &mut imap::Session<T>,
    mailbox: &MailboxConfig,
) -> Result<String, MailError> {
    if let Some(folder) = mailbox.imap.sent_folder.as_deref() {
        return Ok(folder.trim().to_string());
    }
    let names = session
        .list(None, Some("*"))
        .map_err(|error| MailError::Imap(error.to_string()))?;
    let matches = names
        .iter()
        .filter(|name| {
            name.attributes().iter().any(|attribute| {
                matches!(attribute, NameAttribute::Custom(value) if value.eq_ignore_ascii_case("\\Sent"))
            })
        })
        .map(|name| name.name().to_string())
        .collect::<Vec<_>>();
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

fn sent_uids<T: Read + Write>(
    session: &mut imap::Session<T>,
    last_uid: u64,
    cutoff: i64,
) -> Result<Vec<u32>, MailError> {
    let Some(query) = sent_uid_query(last_uid, cutoff)? else {
        return Ok(Vec::new());
    };
    uid_search(session, &query)
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

fn fetch_unseen_with_session<T: Read + Write>(
    mut session: imap::Session<T>,
    mailbox: &MailboxConfig,
    limit: usize,
) -> Result<Vec<InboundMessage>, MailError> {
    let selected = session
        .select(&mailbox.imap.folder)
        .map_err(|error| MailError::Imap(error.to_string()))?;
    let uid_validity = selected.uid_validity.unwrap_or_default() as u64;
    let mut uids = search_unseen_uids(&mut session, mailbox)?;
    uids.sort_unstable();

    let mut messages = Vec::new();
    for uid in uids {
        if messages.len() >= limit {
            break;
        }
        let fetches = session
            .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
            .map_err(|error| MailError::Imap(error.to_string()))?;
        for fetch in fetches.iter() {
            let raw = fetch
                .body()
                .ok_or_else(|| MailError::Imap(format!("message uid {uid} had no body")))?;
            let fetched_uid = fetch.uid.unwrap_or(uid) as u64;
            let message = parse_inbound_message(&mailbox.id, uid_validity, fetched_uid, raw)?;
            if message_matches_accepted_conditions(&message, &mailbox.accepted_conditions) {
                messages.push(message);
                if messages.len() >= limit {
                    break;
                }
            }
        }
    }
    let _ = session.logout();
    Ok(messages)
}

fn search_unseen_uids<T: Read + Write>(
    session: &mut imap::Session<T>,
    mailbox: &MailboxConfig,
) -> Result<Vec<u32>, MailError> {
    if let Some(queries) = recipient_prefilter_queries(mailbox) {
        let mut uids = Vec::new();
        for query in queries {
            for uid in uid_search(session, &query)? {
                if !uids.contains(&uid) {
                    uids.push(uid);
                }
            }
        }
        return Ok(uids);
    }
    uid_search(session, "UNSEEN")
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

fn uid_search<T: Read + Write>(
    session: &mut imap::Session<T>,
    query: &str,
) -> Result<Vec<u32>, MailError> {
    Ok(session
        .uid_search(query)
        .map_err(|error| MailError::Imap(error.to_string()))?
        .into_iter()
        .collect::<Vec<_>>())
}

fn imap_search_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub(crate) fn mark_seen_blocking(mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
    if mailbox.imap.tls {
        let session = login_imap_tls(mailbox)?;
        mark_seen_with_session(session, mailbox, uid)
    } else {
        let session = login_imap_plain(mailbox)?;
        mark_seen_with_session(session, mailbox, uid)
    }
}

fn mark_seen_with_session<T: Read + Write>(
    mut session: imap::Session<T>,
    mailbox: &MailboxConfig,
    uid: u64,
) -> Result<(), MailError> {
    session
        .select(&mailbox.imap.folder)
        .map_err(|error| MailError::Imap(error.to_string()))?;
    session
        .uid_store(uid.to_string(), "+FLAGS (\\Seen)")
        .map_err(|error| MailError::Imap(error.to_string()))?;
    let _ = session.logout();
    Ok(())
}

fn login_imap_tls(
    mailbox: &MailboxConfig,
) -> Result<imap::Session<native_tls::TlsStream<TcpStream>>, MailError> {
    let address = (mailbox.imap.host.as_str(), mailbox.imap.port);
    let tls = TlsConnector::builder()
        .build()
        .map_err(|error| MailError::Imap(error.to_string()))?;
    let client = imap::connect(address, &mailbox.imap.host, &tls)
        .map_err(|error| MailError::Imap(error.to_string()))?;
    client
        .login(&mailbox.imap.username, &mailbox.imap.password)
        .map_err(|(error, _)| MailError::Imap(error.to_string()))
}

fn login_imap_plain(mailbox: &MailboxConfig) -> Result<imap::Session<TcpStream>, MailError> {
    let address = (mailbox.imap.host.as_str(), mailbox.imap.port);
    let stream = TcpStream::connect(address).map_err(|error| MailError::Imap(error.to_string()))?;
    let mut client = imap::Client::new(stream);
    client
        .read_greeting()
        .map_err(|error| MailError::Imap(error.to_string()))?;
    client
        .login(&mailbox.imap.username, &mailbox.imap.password)
        .map_err(|(error, _)| MailError::Imap(error.to_string()))
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
mod tests {
    use crate::config::{AcceptedCondition, AgentConfig, ImapConfig, MailboxConfig, SmtpConfig};
    use crate::mail::{ComposedEmail, OutboundAction, OutboundActionKind};

    use super::*;

    #[test]
    fn recipient_prefilter_queries_use_common_delivery_headers() {
        let mut mailbox = mailbox_config();
        mailbox.accepted_conditions = vec![AcceptedCondition {
            recipients: vec!["Support@example.com".to_string()],
            subject_regex: vec!["(?i)billing".to_string()],
        }];

        let queries = recipient_prefilter_queries(&mailbox).unwrap();

        assert_eq!(
            queries,
            vec![
                "UNSEEN HEADER To \"support@example.com\"".to_string(),
                "UNSEEN HEADER Cc \"support@example.com\"".to_string(),
                "UNSEEN HEADER Delivered-To \"support@example.com\"".to_string(),
                "UNSEEN HEADER X-Original-To \"support@example.com\"".to_string(),
                "UNSEEN HEADER Envelope-To \"support@example.com\"".to_string(),
            ]
        );
    }

    #[test]
    fn subject_only_conditions_do_not_build_recipient_prefilter_queries() {
        let mut mailbox = mailbox_config();
        mailbox.accepted_conditions = vec![AcceptedCondition {
            recipients: vec![],
            subject_regex: vec!["urgent".to_string()],
        }];

        assert_eq!(recipient_prefilter_queries(&mailbox), None);
    }

    #[test]
    fn sent_uid_query_uses_non_reversing_upper_bound() {
        assert_eq!(
            sent_uid_query(42, 0).unwrap().as_deref(),
            Some("UID 43:4294967295 SINCE 01-Jan-1970")
        );
        assert_eq!(sent_uid_query(u32::MAX as u64, 0).unwrap(), None);
    }

    #[test]
    fn sent_sync_range_restarts_when_backfill_expands() {
        let cursor = SentSyncCursor {
            folder_name: "Sent".to_string(),
            uid_validity: 7,
            last_uid: 900,
            backfill_cutoff: 2_000,
        };

        assert_eq!(
            sent_sync_range(Some(&cursor), "Sent", 7, 2_100),
            (900, 2_000)
        );
        assert_eq!(sent_sync_range(Some(&cursor), "Sent", 7, 1_000), (0, 1_000));
    }

    #[test]
    fn build_message_sets_reply_thread_headers() {
        let smtp = SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            starttls: true,
            username: "support@example.com".to_string(),
            password: "secret".to_string(),
            from: "support@example.com".to_string(),
        };
        let action = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            html_body: None,
            reason: "known answer".to_string(),
            reply_to: Some("remote@example.com".to_string()),
            message_id: Some("<reply@example.com>".to_string()),
            in_reply_to: Some("<inbound@example.com>".to_string()),
            references: vec![
                "<root@example.com>".to_string(),
                "<inbound@example.com>".to_string(),
            ],
        };

        let message = build_message(&smtp, &action).unwrap();
        let rendered = String::from_utf8(message.formatted()).unwrap();

        assert!(rendered.contains("Message-ID: <reply@example.com>\r\n"));
        assert!(rendered.contains("Reply-To: remote@example.com\r\n"));
        assert!(rendered.contains("In-Reply-To: <inbound@example.com>\r\n"));
        assert!(rendered.contains("References: <root@example.com> <inbound@example.com>\r\n"));
    }

    #[test]
    fn build_message_uses_html_body_as_single_html_part() {
        let smtp = SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            starttls: true,
            username: "support@example.com".to_string(),
            password: "secret".to_string(),
            from: "support@example.com".to_string(),
        };
        let action = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            html_body: Some("<p>Answer</p>".to_string()),
            reason: "known answer".to_string(),
            reply_to: None,
            message_id: Some("<reply@example.com>".to_string()),
            in_reply_to: Some("<inbound@example.com>".to_string()),
            references: vec![],
        };

        let message = build_message(&smtp, &action).unwrap();
        let rendered = String::from_utf8(message.formatted()).unwrap();

        assert!(rendered.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(rendered.contains("<p>Answer</p>"));
        assert!(!rendered.contains("multipart/alternative"));
        assert!(!rendered.contains("Content-Type: text/plain"));
    }

    #[test]
    fn build_composed_message_uses_multipart_and_hides_bcc_header() {
        let smtp = SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            starttls: true,
            username: "support@example.com".to_string(),
            password: "secret".to_string(),
            from: "support@example.com".to_string(),
        };
        let message = ComposedEmail {
            to: vec!["person@example.com".to_string()],
            cc: vec!["copy@example.com".to_string()],
            bcc: vec!["hidden@example.com".to_string()],
            subject: "Re: Question".to_string(),
            text_body: "Answer".to_string(),
            html_body: Some("<p>Answer</p>".to_string()),
            message_id: Some("<reply@example.com>".to_string()),
            in_reply_to: Some("<inbound@example.com>".to_string()),
            references: vec!["<inbound@example.com>".to_string()],
        };

        let rendered =
            String::from_utf8(build_composed_message(&smtp, &message).unwrap().formatted())
                .unwrap();

        assert!(rendered.contains("Content-Type: multipart/alternative;"));
        assert!(rendered.contains("To: person@example.com\r\n"));
        assert!(rendered.contains("Cc: copy@example.com\r\n"));
        assert!(!rendered.contains("Bcc: hidden@example.com"));
        assert!(rendered.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(rendered.contains("Content-Type: text/html; charset=utf-8\r\n"));
    }

    fn mailbox_config() -> MailboxConfig {
        MailboxConfig {
            id: "support".to_string(),
            address: "support@example.com".to_string(),
            enabled: true,
            poll_interval_seconds: 30,
            safety_forward_to: vec!["safety@example.com".to_string()],
            signature: None,
            accepted_conditions: vec![],
            mcp_servers: vec![],
            agent: AgentConfig {
                system_prompt_path: "agent.md".into(),
                default_forward_to: vec!["human@example.com".to_string()],
            },
            imap: ImapConfig {
                host: "imap.example.com".to_string(),
                port: 993,
                tls: true,
                username: "support@example.com".to_string(),
                password: "secret".to_string(),
                folder: "INBOX".to_string(),
                sent_folder: None,
                sent_backfill_days: 0,
            },
            smtp: SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 587,
                starttls: true,
                username: "support@example.com".to_string(),
                password: "secret".to_string(),
                from: "support@example.com".to_string(),
            },
        }
    }
}
