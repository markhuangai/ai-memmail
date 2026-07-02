use async_trait::async_trait;
use mailparse::MailHeaderMap;
use serde::{Deserialize, Serialize};

use crate::config::{MailboxConfig, SmtpConfig};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DedupeKey {
    pub mailbox_id: String,
    pub uid_validity: u64,
    pub uid: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageMetadata {
    pub mailbox_id: String,
    pub uid_validity: u64,
    pub uid: u64,
    pub message_id: Option<String>,
    pub from_addr: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutboundActionKind {
    Reply,
    Forward,
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboundAction {
    pub kind: OutboundActionKind,
    pub recipients: Vec<String>,
    pub subject: String,
    pub body: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundMessage {
    pub metadata: MessageMetadata,
    pub plain_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("mail task failed: {0}")]
    Task(String),
    #[error("imap error: {0}")]
    Imap(String),
    #[error("smtp error: {0}")]
    Smtp(String),
    #[error("message parse error: {0}")]
    Parse(String),
    #[error("message build error: {0}")]
    Build(String),
}

#[async_trait]
pub trait MailTransport: Send + Sync {
    async fn fetch_unseen(
        &self,
        mailbox: &MailboxConfig,
        limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError>;

    async fn send(&self, smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError>;

    async fn mark_seen(&self, mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemMailClient;

pub trait BlockingMailClient: Send + Sync + Clone + 'static {
    fn fetch_unseen(
        &self,
        mailbox: &MailboxConfig,
        limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError>;

    fn send(&self, smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError>;

    fn mark_seen(&self, mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError>;
}

impl BlockingMailClient for SystemMailClient {
    fn fetch_unseen(
        &self,
        mailbox: &MailboxConfig,
        limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError> {
        crate::mail_external::fetch_unseen_blocking(mailbox, limit)
    }

    fn send(&self, smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
        crate::mail_external::send_blocking(smtp, action)
    }

    fn mark_seen(&self, mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
        crate::mail_external::mark_seen_blocking(mailbox, uid)
    }
}

#[derive(Debug, Clone)]
pub struct LiveMailTransport<C = SystemMailClient> {
    client: C,
}

impl Default for LiveMailTransport<SystemMailClient> {
    fn default() -> Self {
        Self {
            client: SystemMailClient,
        }
    }
}

impl<C> LiveMailTransport<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

#[async_trait]
impl<C> MailTransport for LiveMailTransport<C>
where
    C: BlockingMailClient,
{
    async fn fetch_unseen(
        &self,
        mailbox: &MailboxConfig,
        limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError> {
        let client = self.client.clone();
        let mailbox = mailbox.clone();
        tokio::task::spawn_blocking(move || client.fetch_unseen(&mailbox, limit))
            .await
            .map_err(|error| MailError::Task(error.to_string()))?
    }

    async fn send(&self, smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
        let client = self.client.clone();
        let smtp = smtp.clone();
        let action = action.clone();
        tokio::task::spawn_blocking(move || client.send(&smtp, &action))
            .await
            .map_err(|error| MailError::Task(error.to_string()))?
    }

    async fn mark_seen(&self, mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
        let client = self.client.clone();
        let mailbox = mailbox.clone();
        tokio::task::spawn_blocking(move || client.mark_seen(&mailbox, uid))
            .await
            .map_err(|error| MailError::Task(error.to_string()))?
    }
}

impl MessageMetadata {
    pub fn dedupe_key(&self) -> DedupeKey {
        DedupeKey {
            mailbox_id: self.mailbox_id.clone(),
            uid_validity: self.uid_validity,
            uid: self.uid,
        }
    }
}

pub fn parse_inbound_message(
    mailbox_id: &str,
    uid_validity: u64,
    uid: u64,
    raw: &[u8],
) -> Result<InboundMessage, MailError> {
    let parsed = mailparse::parse_mail(raw).map_err(|error| MailError::Parse(error.to_string()))?;
    let from_addr = parsed.headers.get_first_value("From").unwrap_or_default();
    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_default();
    let message_id = parsed.headers.get_first_value("Message-ID");
    let plain_text = extract_plain_text(&parsed)?;
    Ok(InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: mailbox_id.to_string(),
            uid_validity,
            uid,
            message_id,
            from_addr,
            subject,
        },
        plain_text,
    })
}

pub fn validate_outbound_action(action: &OutboundAction) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    match action.kind {
        OutboundActionKind::Reply | OutboundActionKind::Forward => {
            if action.recipients.is_empty() {
                errors.push(ValidationError {
                    field: "recipients".to_string(),
                    message: "at least one recipient is required".to_string(),
                });
            }
            if action.subject.trim().is_empty() {
                errors.push(ValidationError {
                    field: "subject".to_string(),
                    message: "subject is required".to_string(),
                });
            }
            if action.body.trim().is_empty() {
                errors.push(ValidationError {
                    field: "body".to_string(),
                    message: "body is required".to_string(),
                });
            }
        }
        OutboundActionKind::Noop => {
            if !action.recipients.is_empty() {
                errors.push(ValidationError {
                    field: "recipients".to_string(),
                    message: "noop must not define recipients".to_string(),
                });
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn forward_body(intro: &str, message: &InboundMessage) -> String {
    let message_id = message.metadata.message_id.as_deref().unwrap_or("(none)");
    format!(
        "{intro}\n\n---------- Forwarded message ---------\nFrom: {}\nSubject: {}\nMessage-ID: {message_id}\nUID: {}:{}\n\n{}",
        message.metadata.from_addr,
        message.metadata.subject,
        message.metadata.uid_validity,
        message.metadata.uid,
        message.plain_text
    )
}

fn extract_plain_text(parsed: &mailparse::ParsedMail<'_>) -> Result<String, MailError> {
    if parsed.ctype.mimetype.eq_ignore_ascii_case("text/plain") {
        return parsed
            .get_body()
            .map_err(|error| MailError::Parse(error.to_string()));
    }
    for part in &parsed.subparts {
        if let Ok(body) = extract_plain_text(part) {
            if !body.trim().is_empty() {
                return Ok(body);
            }
        }
    }
    parsed
        .get_body()
        .map_err(|error| MailError::Parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::config::{AgentConfig, ImapConfig, MailboxConfig, SmtpConfig};

    use super::*;

    #[derive(Clone, Default)]
    struct FakeBlockingMailClient {
        state: Arc<Mutex<FakeBlockingMailState>>,
    }

    #[derive(Default)]
    struct FakeBlockingMailState {
        fetched_limits: Vec<usize>,
        sent_subjects: Vec<String>,
        seen_uids: Vec<u64>,
    }

    impl BlockingMailClient for FakeBlockingMailClient {
        fn fetch_unseen(
            &self,
            mailbox: &MailboxConfig,
            limit: usize,
        ) -> Result<Vec<InboundMessage>, MailError> {
            self.state
                .lock()
                .expect("fake mail state")
                .fetched_limits
                .push(limit);
            Ok(vec![InboundMessage {
                metadata: MessageMetadata {
                    mailbox_id: mailbox.id.clone(),
                    uid_validity: 9,
                    uid: 42,
                    message_id: Some("<m1@example.com>".to_string()),
                    from_addr: "sender@example.com".to_string(),
                    subject: "Question".to_string(),
                },
                plain_text: "Body".to_string(),
            }])
        }

        fn send(&self, _smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
            self.state
                .lock()
                .expect("fake mail state")
                .sent_subjects
                .push(action.subject.clone());
            Ok(())
        }

        fn mark_seen(&self, _mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
            self.state
                .lock()
                .expect("fake mail state")
                .seen_uids
                .push(uid);
            Ok(())
        }
    }

    #[tokio::test]
    async fn live_transport_delegates_to_blocking_client() {
        let client = FakeBlockingMailClient::default();
        let transport = LiveMailTransport::new(client.clone());
        let mailbox = mailbox_config();
        let action = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            reason: "test".to_string(),
        };

        let fetched = transport.fetch_unseen(&mailbox, 3).await.unwrap();
        transport.send(&mailbox.smtp, &action).await.unwrap();
        transport.mark_seen(&mailbox, 42).await.unwrap();

        assert_eq!(fetched[0].metadata.mailbox_id, "support");
        let state = client.state.lock().expect("fake mail state");
        assert_eq!(state.fetched_limits, vec![3]);
        assert_eq!(state.sent_subjects, vec!["Re: Question"]);
        assert_eq!(state.seen_uids, vec![42]);
    }

    #[test]
    fn metadata_builds_stable_dedupe_key() {
        let metadata = MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 7,
            uid: 42,
            message_id: None,
            from_addr: "a@example.com".to_string(),
            subject: "Hello".to_string(),
        };
        assert_eq!(
            metadata.dedupe_key(),
            DedupeKey {
                mailbox_id: "support".to_string(),
                uid_validity: 7,
                uid: 42
            }
        );
    }

    #[test]
    fn validates_reply_requires_recipient_subject_and_body() {
        let action = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec![],
            subject: "".to_string(),
            body: "".to_string(),
            reason: "test".to_string(),
        };
        let errors = validate_outbound_action(&action).unwrap_err();
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn validates_noop_has_no_recipients() {
        let action = OutboundAction {
            kind: OutboundActionKind::Noop,
            recipients: vec!["person@example.com".to_string()],
            subject: "".to_string(),
            body: "".to_string(),
            reason: "nothing to do".to_string(),
        };
        let errors = validate_outbound_action(&action).unwrap_err();
        assert_eq!(errors[0].field, "recipients");
    }

    #[test]
    fn validates_complete_reply_forward_and_noop_actions() {
        let reply = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Hello".to_string(),
            body: "Thanks".to_string(),
            reason: "known answer".to_string(),
        };
        assert!(validate_outbound_action(&reply).is_ok());

        let forward = OutboundAction {
            kind: OutboundActionKind::Forward,
            recipients: vec!["human@example.com".to_string()],
            subject: "Review".to_string(),
            body: "Please review".to_string(),
            reason: "needs human review".to_string(),
        };
        assert!(validate_outbound_action(&forward).is_ok());

        let noop = OutboundAction {
            kind: OutboundActionKind::Noop,
            recipients: vec![],
            subject: "".to_string(),
            body: "".to_string(),
            reason: "nothing safe to do".to_string(),
        };
        assert!(validate_outbound_action(&noop).is_ok());
    }

    #[test]
    fn parses_simple_inbound_message() {
        let raw = b"From: Sender <sender@example.com>\r\nSubject: Hello\r\nMessage-ID: <m1@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody text";
        let message = parse_inbound_message("support", 9, 10, raw).unwrap();
        assert_eq!(message.metadata.mailbox_id, "support");
        assert_eq!(message.metadata.uid_validity, 9);
        assert_eq!(message.metadata.uid, 10);
        assert_eq!(
            message.metadata.message_id,
            Some("<m1@example.com>".to_string())
        );
        assert_eq!(message.metadata.subject, "Hello");
        assert_eq!(message.plain_text, "Body text");
    }

    #[test]
    fn parses_text_part_from_multipart_message() {
        let raw = b"From: sender@example.com\r\nSubject: Multipart\r\nContent-Type: multipart/alternative; boundary=abc\r\n\r\n--abc\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n--abc\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML body</p>\r\n--abc--";
        let message = parse_inbound_message("support", 1, 2, raw).unwrap();
        assert_eq!(message.plain_text.trim(), "Plain body");
    }

    #[test]
    fn forward_body_includes_metadata_and_text() {
        let message = InboundMessage {
            metadata: MessageMetadata {
                mailbox_id: "support".to_string(),
                uid_validity: 1,
                uid: 2,
                message_id: Some("<m1@example.com>".to_string()),
                from_addr: "person@example.com".to_string(),
                subject: "Question".to_string(),
            },
            plain_text: "Original text".to_string(),
        };
        let body = forward_body("Intro", &message);
        assert_eq!(
            body,
            "Intro\n\n---------- Forwarded message ---------\nFrom: person@example.com\nSubject: Question\nMessage-ID: <m1@example.com>\nUID: 1:2\n\nOriginal text"
        );
    }

    #[test]
    fn send_blocking_rejects_invalid_action_before_smtp() {
        let smtp = mailbox_config().smtp;
        let action = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec![],
            subject: "".to_string(),
            body: "".to_string(),
            reason: "invalid".to_string(),
        };
        let error = crate::mail_external::send_blocking(&smtp, &action).unwrap_err();
        assert!(error.to_string().contains("recipients"));
        assert!(error.to_string().contains("subject"));
        assert!(error.to_string().contains("body"));
    }

    #[test]
    fn parse_mailbox_reports_invalid_addresses() {
        assert!(crate::mail_external::parse_mailbox("support@example.com").is_ok());
        assert!(crate::mail_external::parse_mailbox("not an address").is_err());
    }

    fn mailbox_config() -> MailboxConfig {
        MailboxConfig {
            id: "support".to_string(),
            address: "support@example.com".to_string(),
            enabled: true,
            poll_interval_seconds: 30,
            safety_forward_to: vec!["safety@example.com".to_string()],
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
