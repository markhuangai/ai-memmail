use super::{outbound_message_id, MailError, OutboundAction, OutboundActionKind};
use crate::config::MailboxConfig;

use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadMessage {
    pub direction: MessageDirection,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from_addr: String,
    pub recipients: Vec<String>,
    pub subject: String,
    pub authored_text: String,
    pub body_truncated: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadContext {
    pub thread_id: String,
    pub messages: Vec<ThreadMessage>,
}

impl ThreadContext {
    pub fn empty(thread_id: String) -> Self {
        Self {
            thread_id,
            messages: Vec::new(),
        }
    }

    pub fn has_truncated_body(&self) -> bool {
        self.messages.iter().any(|message| message.body_truncated)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteExtraction {
    pub authored_text: String,
    pub quoted_text_removed: bool,
}

pub fn extract_authored_text(body: &str, canonical_history_exists: bool) -> QuoteExtraction {
    if !canonical_history_exists {
        return unchanged(body);
    }
    let marker = quote_marker_index(body);
    let Some(index) = marker else {
        return unchanged(body);
    };
    let authored = body[..index].trim();
    if authored.is_empty() {
        return unchanged(body);
    }
    QuoteExtraction {
        authored_text: authored.to_string(),
        quoted_text_removed: true,
    }
}

fn unchanged(body: &str) -> QuoteExtraction {
    QuoteExtraction {
        authored_text: body.to_string(),
        quoted_text_removed: false,
    }
}

fn quote_marker_index(body: &str) -> Option<usize> {
    let patterns = [
        r"(?ms)^On .{1,1000}?wrote:\s*$",
        r"(?mi)^-{2,}\s*Original Message\s*-{2,}\s*$",
        r"(?m)^\s*>",
        r"(?mi)^From: .+\r?\nSent: .+\r?\nTo: .+\r?\nSubject: .+$",
    ];
    patterns
        .iter()
        .filter_map(|pattern| {
            Regex::new(pattern)
                .ok()?
                .find(body)
                .map(|found| found.start())
        })
        .min()
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

pub(crate) fn thread_handoff_action(
    mailbox: &MailboxConfig,
    thread_context: &ThreadContext,
    destination: &str,
    remote_target: &str,
) -> Result<OutboundAction, MailError> {
    let latest = thread_context
        .messages
        .iter()
        .rev()
        .find(|message| message.direction == MessageDirection::Inbound)
        .ok_or_else(|| MailError::Build("thread handoff has no inbound message".to_string()))?;
    let mut references = latest.references.clone();
    if let Some(message_id) = &latest.message_id {
        if !references.iter().any(|reference| reference == message_id) {
            references.push(message_id.clone());
        }
    }
    Ok(OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: vec![destination.to_string()],
        subject: latest.subject.clone(),
        body: thread_handoff_body(thread_context)?,
        html_body: None,
        reason: "thread handed off for manual handling".to_string(),
        reply_to: Some(remote_target.to_string()),
        message_id: Some(outbound_message_id(mailbox)),
        in_reply_to: latest.message_id.clone(),
        references,
    })
}
