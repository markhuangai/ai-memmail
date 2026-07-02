use std::io::{Read, Write};
use std::net::TcpStream;

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use native_tls::TlsConnector;

use crate::config::{MailboxConfig, SmtpConfig};
use crate::mail::{
    parse_inbound_message, validate_outbound_action, InboundMessage, MailError, OutboundAction,
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

fn fetch_unseen_with_session<T: Read + Write>(
    mut session: imap::Session<T>,
    mailbox: &MailboxConfig,
    limit: usize,
) -> Result<Vec<InboundMessage>, MailError> {
    let selected = session
        .select(&mailbox.imap.folder)
        .map_err(|error| MailError::Imap(error.to_string()))?;
    let uid_validity = selected.uid_validity.unwrap_or_default() as u64;
    let mut uids = session
        .uid_search("UNSEEN")
        .map_err(|error| MailError::Imap(error.to_string()))?
        .into_iter()
        .collect::<Vec<_>>();
    uids.sort_unstable();
    uids.truncate(limit);

    let mut messages = Vec::new();
    for uid in uids {
        let fetches = session
            .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
            .map_err(|error| MailError::Imap(error.to_string()))?;
        for fetch in fetches.iter() {
            let raw = fetch
                .body()
                .ok_or_else(|| MailError::Imap(format!("message uid {uid} had no body")))?;
            let fetched_uid = fetch.uid.unwrap_or(uid) as u64;
            messages.push(parse_inbound_message(
                &mailbox.id,
                uid_validity,
                fetched_uid,
                raw,
            )?);
        }
    }
    let _ = session.logout();
    Ok(messages)
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

    let message = build_message(smtp, action)?;
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
    if !action.references.is_empty() {
        builder = builder.references(action.references.join(" "));
    }
    for recipient in &action.recipients {
        builder = builder.to(parse_mailbox(recipient)?);
    }
    builder
        .body(action.body.clone())
        .map_err(|error| MailError::Build(error.to_string()))
}

pub(crate) fn parse_mailbox(value: &str) -> Result<Mailbox, MailError> {
    value
        .parse::<Mailbox>()
        .map_err(|error| MailError::Build(error.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::mail::{OutboundAction, OutboundActionKind};

    use super::*;

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
            reason: "known answer".to_string(),
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
        assert!(rendered.contains("In-Reply-To: <inbound@example.com>\r\n"));
        assert!(rendered.contains("References: <root@example.com> <inbound@example.com>\r\n"));
    }
}
