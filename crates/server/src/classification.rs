use serde::{Deserialize, Serialize};

use crate::mail::OutboundActionKind;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailCategory {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailTopic {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailTaxonomy {
    pub categories: Vec<EmailCategory>,
    pub topics: Vec<EmailTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailClassificationConfig {
    pub categories: Vec<EmailCategory>,
    pub topics: Vec<EmailTopic>,
    pub rules: Vec<EmailRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailClassification {
    pub category: String,
    #[serde(default)]
    pub topics: Vec<String>,
    pub reason: String,
    pub confidence: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedEmailClassification {
    pub category_id: i64,
    pub category: String,
    pub topic_ids: Vec<i64>,
    pub topics: Vec<String>,
    pub reason: String,
    pub confidence: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailRule {
    pub id: i64,
    pub mailbox_id: String,
    pub name: String,
    pub category_id: i64,
    pub category: String,
    #[serde(default)]
    pub topic_ids: Vec<i64>,
    #[serde(default)]
    pub topics: Vec<String>,
    pub action: EmailRuleAction,
    pub reply_goal: String,
    pub enabled: bool,
    pub priority: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewEmailRule {
    pub mailbox_id: String,
    pub name: String,
    pub category_id: i64,
    #[serde(default)]
    pub topic_ids: Vec<i64>,
    pub action: EmailRuleAction,
    pub reply_goal: String,
    pub enabled: bool,
    pub priority: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailRuleAction {
    Reply,
    Forward,
    Noop,
}

impl EmailRuleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Forward => "forward",
            Self::Noop => "noop",
        }
    }

    pub fn outbound_kind(self) -> OutboundActionKind {
        match self {
            Self::Reply => OutboundActionKind::Reply,
            Self::Forward => OutboundActionKind::Forward,
            Self::Noop => OutboundActionKind::Noop,
        }
    }
}

impl TryFrom<&str> for EmailRuleAction {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "reply" => Ok(Self::Reply),
            "forward" => Ok(Self::Forward),
            "noop" => Ok(Self::Noop),
            _ => Err(format!("unknown email rule action {value}")),
        }
    }
}

pub const DEFAULT_MARKETING_REPLY_GOAL: &str = "Politely thank the sender and decline paid marketing, growth, SEO, lead-generation, advertising, PR, or vendor service offers. Say Mark is focused on organic community growth, free/open-source collaboration, and relevant contributors for now. Do not ask follow-up questions.";

pub fn default_categories() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "marketing_vendor",
            "Paid marketing, growth, SEO, lead-generation, advertising, PR, agency, tool, or vendor service outreach.",
        ),
        (
            "greeting",
            "Short hello, introduction, thanks, or relationship maintenance with no substantial ask.",
        ),
        (
            "question",
            "A concrete question about Mark, a project, article, setup, usage, or technical direction.",
        ),
        (
            "project_opportunity",
            "A collaboration, contribution, integration, partnership, investment, job, speaking, or project opportunity that may need Mark's judgment.",
        ),
        (
            "other",
            "Anything that does not clearly fit the other configured categories.",
        ),
    ]
}

pub fn default_topics() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "dense_mem",
            "Dense-Mem: governed AI memory, MCP access, evidence, typed claims/facts, conflicts, recall, and team/profile isolation.",
        ),
        (
            "ai_memmail",
            "ai-memmail: the email-processing agent, control panel, IMAP/SMTP workflow, history, prompts, and rules.",
        ),
        (
            "gitvibe",
            "GitVibe: maintainer-gated AI development automation for GitHub issues, PRs, labels, workflows, and reviews.",
        ),
        (
            "agentool",
            "agentool: production-ready Vercel AI SDK tools for agents, file operations, shell, search, memory, and context compaction.",
        ),
        (
            "ai_memory",
            "AI memory, RAG limits, graph-backed recall, provenance, conflict handling, retrieval policy, and durable assistant context.",
        ),
        ("general", "General or unclear topic."),
    ]
}

pub fn normalize_label_name(value: &str) -> String {
    let mut output = String::new();
    let mut previous_separator = false;
    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            output.push(character);
            previous_separator = false;
        } else if !previous_separator {
            output.push('_');
            previous_separator = true;
        }
    }
    let normalized = output.trim_matches('_').to_string();
    if normalized.is_empty() {
        "other".to_string()
    } else {
        normalized
    }
}

pub fn valid_confidence(value: u8) -> bool {
    value <= 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_label_names_for_storage() {
        assert_eq!(normalize_label_name("Marketing Vendor"), "marketing_vendor");
        assert_eq!(normalize_label_name(" Dense-Mem / AI "), "dense_mem_ai");
        assert_eq!(normalize_label_name(""), "other");
    }

    #[test]
    fn default_taxonomy_contains_required_labels() {
        assert!(default_categories()
            .iter()
            .any(|(name, _)| *name == "marketing_vendor"));
        assert!(default_topics()
            .iter()
            .any(|(name, _)| *name == "dense_mem"));
    }

    #[test]
    fn rule_actions_map_to_outbound_kinds() {
        assert_eq!(
            EmailRuleAction::Reply.outbound_kind(),
            OutboundActionKind::Reply
        );
        assert_eq!(EmailRuleAction::Forward.as_str(), "forward");
        assert_eq!(
            EmailRuleAction::Forward.outbound_kind(),
            OutboundActionKind::Forward
        );
        assert_eq!(
            EmailRuleAction::Noop.outbound_kind(),
            OutboundActionKind::Noop
        );
        assert_eq!(EmailRuleAction::Noop.as_str(), "noop");
    }

    #[test]
    fn parses_rule_actions_from_storage_values() {
        assert_eq!(
            EmailRuleAction::try_from("reply"),
            Ok(EmailRuleAction::Reply)
        );
        assert_eq!(
            EmailRuleAction::try_from("forward"),
            Ok(EmailRuleAction::Forward)
        );
        assert_eq!(EmailRuleAction::try_from("noop"), Ok(EmailRuleAction::Noop));
        assert!(EmailRuleAction::try_from("archive")
            .unwrap_err()
            .contains("unknown email rule action"));
    }

    #[test]
    fn confidence_must_be_percentage() {
        assert!(valid_confidence(0));
        assert!(valid_confidence(100));
    }
}
