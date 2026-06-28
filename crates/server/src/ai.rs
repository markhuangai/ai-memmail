use serde::{Deserialize, Serialize};

use crate::mail::{validate_outbound_action, OutboundAction, ValidationError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDecision {
    #[serde(flatten)]
    pub action: OutboundAction,
    pub safety_notes: String,
}

pub fn parse_agent_decision(raw: &str) -> Result<AgentDecision, String> {
    let decision: AgentDecision =
        serde_json::from_str(raw).map_err(|error| format!("invalid JSON decision: {error}"))?;
    validate_agent_decision(&decision).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| format!("{}: {}", error.field, error.message))
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    Ok(decision)
}

pub fn validate_agent_decision(decision: &AgentDecision) -> Result<(), Vec<ValidationError>> {
    let mut errors = match validate_outbound_action(&decision.action) {
        Ok(()) => Vec::new(),
        Err(errors) => errors,
    };
    if decision.safety_notes.trim().is_empty() {
        errors.push(ValidationError {
            field: "safety_notes".to_string(),
            message: "safety_notes is required".to_string(),
        });
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
    fn parses_valid_reply_decision() {
        let raw = r#"{
            "kind":"reply",
            "recipients":["user@example.com"],
            "subject":"Re: Hello",
            "body":"Thanks",
            "reason":"Known answer",
            "safety_notes":"No sensitive data"
        }"#;
        let decision = parse_agent_decision(raw).unwrap();
        assert_eq!(decision.action.recipients, vec!["user@example.com"]);
    }

    #[test]
    fn rejects_missing_safety_notes() {
        let raw = r#"{
            "kind":"noop",
            "recipients":[],
            "subject":"",
            "body":"",
            "reason":"No action",
            "safety_notes":""
        }"#;
        let error = parse_agent_decision(raw).unwrap_err();
        assert!(error.contains("safety_notes"));
    }
}
