use super::{ConfigError, EmailSignatureFormat, MailboxConfig};
use crate::html_sanitizer::sanitize_email_html;

pub(super) fn validate_mailbox_signature(mailbox: &MailboxConfig) -> Result<(), ConfigError> {
    let Some(signature) = &mailbox.signature else {
        return Ok(());
    };
    if signature.content.trim().is_empty() {
        return Err(ConfigError::Invalid(format!(
            "mailbox {} signature.content must not be empty",
            mailbox.id
        )));
    }
    if matches!(signature.format, EmailSignatureFormat::Html) {
        let sanitized = sanitize_email_html(&signature.content);
        if sanitized.visually_empty {
            return Err(ConfigError::Invalid(format!(
                "mailbox {} signature.content must not be visually empty after sanitizing",
                mailbox.id
            )));
        }
    }
    Ok(())
}
