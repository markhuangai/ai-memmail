use super::ValidationError;
use lettre::message::Mailbox;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedEmail {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub text_body: String,
    pub html_body: Option<String>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

pub fn validate_composed_email(message: &ComposedEmail) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
        errors.push(ValidationError {
            field: "recipients".to_string(),
            message: "at least one recipient is required".to_string(),
        });
    }
    validate_recipients(&message.to, "to", &mut errors);
    validate_recipients(&message.cc, "cc", &mut errors);
    validate_recipients(&message.bcc, "bcc", &mut errors);
    if message.subject.trim().is_empty() {
        errors.push(ValidationError {
            field: "subject".to_string(),
            message: "subject is required".to_string(),
        });
    }
    if message.text_body.trim().is_empty() {
        errors.push(ValidationError {
            field: "text_body".to_string(),
            message: "text_body is required".to_string(),
        });
    }
    if message
        .html_body
        .as_ref()
        .is_some_and(|body| body.trim().is_empty())
    {
        errors.push(ValidationError {
            field: "html_body".to_string(),
            message: "html_body must not be empty when provided".to_string(),
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_recipients(recipients: &[String], field: &str, errors: &mut Vec<ValidationError>) {
    for (index, recipient) in recipients.iter().enumerate() {
        if recipient.trim().is_empty() {
            errors.push(ValidationError {
                field: format!("{field}[{index}]"),
                message: "recipient is required".to_string(),
            });
            continue;
        }
        if recipient.parse::<Mailbox>().is_err() {
            errors.push(ValidationError {
                field: format!("{field}[{index}]"),
                message: "invalid email address".to_string(),
            });
        }
    }
}
