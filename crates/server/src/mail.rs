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
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
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
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
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

    pub fn thread_id(&self) -> String {
        self.references
            .first()
            .or(self.in_reply_to.as_ref())
            .or(self.message_id.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("{}:{}:{}", self.mailbox_id, self.uid_validity, self.uid))
    }
}

pub const AUTOMATED_REPLY_NOTICE: &str = "This automated reply was sent on behalf of this mailbox. If this needs Mark's attention, reply with: escalation to human";

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
    let message_id = parsed
        .headers
        .get_first_value("Message-ID")
        .and_then(|value| first_message_id(&value));
    let in_reply_to = parsed
        .headers
        .get_first_value("In-Reply-To")
        .and_then(|value| first_message_id(&value));
    let references = parsed
        .headers
        .get_first_value("References")
        .map(|value| message_ids(&value))
        .unwrap_or_default();
    let plain_text = extract_plain_text(&parsed)?;
    Ok(InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: mailbox_id.to_string(),
            uid_validity,
            uid,
            message_id,
            in_reply_to,
            references,
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

pub fn automated_reply_body(body: &str) -> String {
    if body.contains(AUTOMATED_REPLY_NOTICE) {
        return body.to_string();
    }
    let trimmed = body.trim_end();
    format!("{trimmed}\n\n--\n{AUTOMATED_REPLY_NOTICE}")
}

pub fn reply_references(metadata: &MessageMetadata) -> Vec<String> {
    let mut references = metadata.references.clone();
    if let Some(message_id) = &metadata.message_id {
        if !references.iter().any(|reference| reference == message_id) {
            references.push(message_id.clone());
        }
    }
    references
}

pub fn reply_recipient(from_addr: &str) -> String {
    match mailparse::addrparse(from_addr) {
        Ok(addresses) => addresses
            .extract_single_info()
            .map(|address| address.addr.trim().to_string())
            .filter(|address| !address.is_empty())
            .unwrap_or_else(|| from_addr.trim().to_string()),
        Err(_) => from_addr.trim().to_string(),
    }
}

fn first_message_id(value: &str) -> Option<String> {
    message_ids(value).into_iter().next()
}

fn message_ids(value: &str) -> Vec<String> {
    match mailparse::msgidparse(value) {
        Ok(ids) => ids.iter().map(|id| format!("<{id}>")).collect::<Vec<_>>(),
        Err(_) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
    }
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
mod tests;
