use crate::config::{EmailSignatureFormat, MailboxConfig};

use super::{OutboundAction, OutboundActionKind};

pub const AUTOMATED_REPLY_NOTICE: &str = "This automated reply was sent on Mark's behalf. If this needs Mark's attention, reply with: escalation to human";

pub fn automated_reply_body(body: &str) -> String {
    if body.contains(AUTOMATED_REPLY_NOTICE) {
        return body.to_string();
    }
    let trimmed = body.trim_end();
    format!("{trimmed}\n\n--\n{AUTOMATED_REPLY_NOTICE}")
}

pub fn apply_reply_signature(mailbox: &MailboxConfig, action: &mut OutboundAction) {
    action.html_body = None;
    if !matches!(action.kind, OutboundActionKind::Reply) {
        return;
    }
    let Some(signature) = &mailbox.signature else {
        action.body = automated_reply_body(&action.body);
        return;
    };
    match signature.format {
        EmailSignatureFormat::PlainText => {
            action.body = plain_reply_body_with_signature(&action.body, &signature.content);
        }
        EmailSignatureFormat::Html => {
            action.html_body = Some(html_reply_body_with_signature(
                &action.body,
                &signature.content,
            ));
        }
    }
}

pub fn plain_reply_body_with_signature(body: &str, signature: &str) -> String {
    format!("{}\n\n{}", body.trim_end(), signature.trim_end())
}

pub fn html_reply_body_with_signature(body: &str, signature_html: &str) -> String {
    format!(
        "{}<br><br>{}",
        plain_text_to_html(body.trim_end()),
        signature_html.trim_end()
    )
}

fn plain_text_to_html(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            '\n' => output.push_str("<br>"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                output.push_str("<br>");
            }
            _ => output.push(character),
        }
    }
    output
}
