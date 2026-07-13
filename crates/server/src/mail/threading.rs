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
