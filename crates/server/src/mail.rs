use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
