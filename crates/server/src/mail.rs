use async_trait::async_trait;
use mailparse::{MailAddr, MailHeaderMap};
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{AcceptedCondition, MailboxConfig, SmtpConfig};

mod signature;
mod threading;

pub use signature::{apply_reply_signature, automated_reply_body, AUTOMATED_REPLY_NOTICE};
pub use threading::{
    extract_authored_text, MessageDirection, QuoteExtraction, ThreadContext, ThreadMessage,
};

pub const ACCEPTED_CONDITION_RECIPIENT_HEADERS: &[&str] =
    &["To", "Cc", "Delivered-To", "X-Original-To", "Envelope-To"];

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
    #[serde(default)]
    pub recipients: Vec<String>,
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
    #[serde(skip)]
    pub html_body: Option<String>,
    pub reason: String,
    #[serde(default)]
    pub reply_to: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentSyncCursor {
    pub folder_name: String,
    pub uid_validity: u64,
    pub last_uid: u64,
    pub backfill_cutoff: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMessage {
    pub message: InboundMessage,
    pub internal_date: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentFetchBatch {
    pub folder_name: String,
    pub uid_validity: u64,
    pub messages: Vec<SentMessage>,
    pub complete: bool,
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

    async fn fetch_sent(
        &self,
        _mailbox: &MailboxConfig,
        _cursor: Option<&SentSyncCursor>,
        _backfill_cutoff: i64,
        _limit: usize,
    ) -> Result<SentFetchBatch, MailError> {
        Err(MailError::Imap(
            "Sent synchronization is unavailable".to_string(),
        ))
    }
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

    fn fetch_sent(
        &self,
        _mailbox: &MailboxConfig,
        _cursor: Option<&SentSyncCursor>,
        _backfill_cutoff: i64,
        _limit: usize,
    ) -> Result<SentFetchBatch, MailError> {
        Err(MailError::Imap(
            "Sent synchronization is unavailable".to_string(),
        ))
    }
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

    fn fetch_sent(
        &self,
        mailbox: &MailboxConfig,
        cursor: Option<&SentSyncCursor>,
        backfill_cutoff: i64,
        limit: usize,
    ) -> Result<SentFetchBatch, MailError> {
        crate::mail_external::fetch_sent_blocking(mailbox, cursor, backfill_cutoff, limit)
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

    async fn fetch_sent(
        &self,
        mailbox: &MailboxConfig,
        cursor: Option<&SentSyncCursor>,
        backfill_cutoff: i64,
        limit: usize,
    ) -> Result<SentFetchBatch, MailError> {
        let client = self.client.clone();
        let mailbox = mailbox.clone();
        let cursor = cursor.cloned();
        tokio::task::spawn_blocking(move || {
            client.fetch_sent(&mailbox, cursor.as_ref(), backfill_cutoff, limit)
        })
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
    let recipients = recipient_addresses(&parsed.headers);
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
            recipients,
            subject,
        },
        plain_text,
    })
}

pub fn message_matches_accepted_conditions(
    message: &InboundMessage,
    conditions: &[AcceptedCondition],
) -> bool {
    if conditions.is_empty() {
        return true;
    }
    conditions
        .iter()
        .any(|condition| accepted_condition_matches(message, condition))
}

pub fn accepted_conditions_can_prefilter_by_recipient(conditions: &[AcceptedCondition]) -> bool {
    !conditions.is_empty()
        && conditions.iter().all(|condition| {
            condition
                .recipients
                .iter()
                .any(|value| !value.trim().is_empty())
        })
}

pub fn accepted_condition_recipient_filter_values(conditions: &[AcceptedCondition]) -> Vec<String> {
    let mut recipients = Vec::new();
    for condition in conditions {
        for recipient in &condition.recipients {
            push_unique_normalized_address(&mut recipients, recipient);
        }
    }
    recipients
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
            if matches!(action.kind, OutboundActionKind::Reply)
                && action
                    .html_body
                    .as_ref()
                    .is_some_and(|body| body.trim().is_empty())
            {
                errors.push(ValidationError {
                    field: "html_body".to_string(),
                    message: "html_body must not be empty when provided".to_string(),
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
    if !matches!(action.kind, OutboundActionKind::Reply) && action.html_body.is_some() {
        errors.push(ValidationError {
            field: "html_body".to_string(),
            message: "html_body is only supported for replies".to_string(),
        });
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

pub fn thread_handoff_body(thread_context: &ThreadContext) -> Result<String, MailError> {
    const HANDOFF_BODY_MAX_BYTES: usize = 5 * 1024 * 1024;

    if thread_context.messages.is_empty() {
        return Err(MailError::Build(
            "thread handoff requires at least one stored message".to_string(),
        ));
    }

    let mut body = String::from(
        "ai-memmail is handing this conversation to a personal inbox.\n\
        Replying to this email should address the latest remote sender.\n\n\
        ---------- Conversation handoff ---------",
    );
    for (index, message) in thread_context.messages.iter().enumerate() {
        if message.body_truncated {
            return Err(MailError::Build(
                "thread handoff cannot use truncated stored message bodies".to_string(),
            ));
        }
        let direction = match message.direction {
            MessageDirection::Inbound => "Inbound",
            MessageDirection::Outbound => "Outbound",
        };
        body.push_str(&format!(
            "\n\n[{}] {direction}\nFrom: {}\nTo: {}\nSubject: {}\nMessage-ID: {}\nIn-Reply-To: {}\nReferences: {}\n\n{}",
            index + 1,
            message.from_addr,
            if message.recipients.is_empty() {
                "(none)".to_string()
            } else {
                message.recipients.join(", ")
            },
            message.subject,
            message.message_id.as_deref().unwrap_or("(none)"),
            message.in_reply_to.as_deref().unwrap_or("(none)"),
            if message.references.is_empty() {
                "(none)".to_string()
            } else {
                message.references.join(" ")
            },
            message.authored_text
        ));
        if body.len() > HANDOFF_BODY_MAX_BYTES {
            return Err(MailError::Build(
                "thread handoff transcript exceeds 5 MiB".to_string(),
            ));
        }
    }
    Ok(body)
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

pub fn outbound_message_id(mailbox: &MailboxConfig) -> String {
    let domain = mailbox
        .smtp
        .from
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim())
        .filter(|domain| !domain.is_empty())
        .unwrap_or("ai-memmail.local");
    format!("<{}@{}>", Uuid::new_v4(), domain)
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

fn accepted_condition_matches(message: &InboundMessage, condition: &AcceptedCondition) -> bool {
    condition_recipients_match(message, condition) && condition_subject_match(message, condition)
}

fn condition_recipients_match(message: &InboundMessage, condition: &AcceptedCondition) -> bool {
    let accepted = condition
        .recipients
        .iter()
        .filter_map(|recipient| normalize_email_address(recipient))
        .collect::<Vec<_>>();
    if accepted.is_empty() {
        return true;
    }
    let message_recipients = message
        .metadata
        .recipients
        .iter()
        .filter_map(|recipient| normalize_email_address(recipient))
        .collect::<Vec<_>>();
    accepted.iter().any(|recipient| {
        message_recipients
            .iter()
            .any(|candidate| candidate == recipient)
    })
}

fn condition_subject_match(message: &InboundMessage, condition: &AcceptedCondition) -> bool {
    if condition.subject_regex.is_empty() {
        return true;
    }
    condition.subject_regex.iter().any(|pattern| {
        Regex::new(pattern).is_ok_and(|regex| regex.is_match(&message.metadata.subject))
    })
}

fn recipient_addresses(headers: &[mailparse::MailHeader<'_>]) -> Vec<String> {
    let mut recipients = Vec::new();
    for header_name in ACCEPTED_CONDITION_RECIPIENT_HEADERS {
        for header in headers.get_all_headers(header_name) {
            match mailparse::addrparse_header(header) {
                Ok(addresses) => {
                    for address in addresses.iter() {
                        collect_mail_addr(address, &mut recipients);
                    }
                }
                Err(_) => {
                    for value in header.get_value().split([',', ';']) {
                        push_unique_normalized_address(&mut recipients, value);
                    }
                }
            }
        }
    }
    recipients
}

fn collect_mail_addr(address: &MailAddr, recipients: &mut Vec<String>) {
    match address {
        MailAddr::Single(info) => push_unique_normalized_address(recipients, &info.addr),
        MailAddr::Group(group) => {
            for info in &group.addrs {
                push_unique_normalized_address(recipients, &info.addr);
            }
        }
    }
}

fn push_unique_normalized_address(recipients: &mut Vec<String>, value: &str) {
    if let Some(address) = normalize_email_address(value) {
        if !recipients.iter().any(|candidate| candidate == &address) {
            recipients.push(address);
        }
    }
}

fn normalize_email_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let candidate = match (trimmed.rfind('<'), trimmed.rfind('>')) {
        (Some(start), Some(end)) if start < end => &trimmed[start + 1..end],
        _ => trimmed,
    };
    let candidate = candidate.trim().trim_matches('"').trim();
    if candidate.contains('@') {
        Some(candidate.to_ascii_lowercase())
    } else {
        None
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
