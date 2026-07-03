use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ai::AgentDecision;
use crate::classification::{
    default_categories, default_topics, normalize_label_name, EmailCategory, EmailClassification,
    EmailRule, EmailRuleAction, EmailTaxonomy, EmailTopic, NewEmailRule,
    ResolvedEmailClassification, DEFAULT_MARKETING_REPLY_GOAL,
};
use crate::config::AppConfig;
use crate::logging::LogLevel;
use crate::mail::{DedupeKey, InboundMessage, OutboundAction, OutboundActionKind};
use crate::safety::SafetyCategory;
pub use crate::storage_pg::PgStore;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("postgres connection failed: {0}")]
    Connect(#[from] tokio_postgres::Error),
    #[error("processing run id is not a uuid: {0}")]
    InvalidRunId(#[from] uuid::Error),
    #[error("applied migration {version} has name {applied_name:?}, expected {expected_name:?}")]
    MigrationNameMismatch {
        version: i32,
        expected_name: &'static str,
        applied_name: String,
    },
    #[error(
        "applied migration {version} checksum mismatch: expected {expected_checksum}, found {applied_checksum}"
    )]
    MigrationChecksumMismatch {
        version: i32,
        expected_checksum: String,
        applied_checksum: String,
    },
    #[error("processing store lock poisoned")]
    LockPoisoned,
    #[error("classification not found: {0}")]
    ClassificationNotFound(String),
    #[error("invalid classification policy: {0}")]
    InvalidClassification(String),
}

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");
pub const HISTORY_BODY_THREADING_SQL: &str =
    include_str!("../migrations/002_history_body_threading.sql");
pub const EMAIL_CLASSIFICATION_RULES_SQL: &str =
    include_str!("../migrations/003_email_classification_rules.sql");
pub const DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL: &str =
    include_str!("../migrations/004_default_email_rule_seed_uniqueness.sql");
pub(crate) const MIGRATION_LOCK_ID: i64 = 4_971_774_501_001;
pub(crate) const SCHEMA_MIGRATIONS_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    checksum TEXT NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
";
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_init",
        sql: INIT_SQL,
    },
    Migration {
        version: 2,
        name: "002_history_body_threading",
        sql: HISTORY_BODY_THREADING_SQL,
    },
    Migration {
        version: 3,
        name: "003_email_classification_rules",
        sql: EMAIL_CLASSIFICATION_RULES_SQL,
    },
    Migration {
        version: 4,
        name: "004_default_email_rule_seed_uniqueness",
        sql: DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL,
    },
];
pub const PROCESSING_STATUS_PROCESSING: &str = "processing";
pub const PROCESSING_STATUS_RETRYABLE_FAILED: &str = "retryable_failed";
pub const PROCESSING_STATUS_SEND_FAILED: &str = "send_failed";
pub const PROCESSING_STALE_AFTER_MINUTES: i32 = 15;
pub const INBOUND_BODY_STORAGE_MAX_CHARS: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingClaim {
    Claimed,
    InProgress { status: String },
    AlreadyFinished { status: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    pub version: i32,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedMigration {
    pub(crate) name: String,
    pub(crate) checksum: String,
}

#[async_trait::async_trait]
pub trait ProcessingStore: Send + Sync {
    async fn claim_message(
        &self,
        run_id: &str,
        message: &InboundMessage,
    ) -> Result<ProcessingClaim, StorageError>;

    async fn update_message_status(
        &self,
        key: &DedupeKey,
        status: &str,
        outbound_action: Option<&OutboundActionKind>,
    ) -> Result<(), StorageError>;

    async fn record_safety_result(
        &self,
        key: &DedupeKey,
        category: &SafetyCategory,
        reason: &str,
    ) -> Result<(), StorageError>;

    async fn upsert_sender_review(
        &self,
        sender: &str,
        mailbox_id: &str,
        reason: &str,
    ) -> Result<(), StorageError>;

    async fn record_agent_decision(
        &self,
        _key: &DedupeKey,
        _decision: &AgentDecision,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn record_outbound_action(
        &self,
        _key: &DedupeKey,
        _action: &OutboundAction,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn record_outbound_review(
        &self,
        _key: &DedupeKey,
        _status: &str,
        _reason: &str,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn ensure_default_classification_policy(
        &self,
        _config: &AppConfig,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn active_email_taxonomy(&self) -> Result<EmailTaxonomy, StorageError> {
        Ok(memory_default_taxonomy())
    }

    async fn resolve_email_classification(
        &self,
        classification: &EmailClassification,
    ) -> Result<ResolvedEmailClassification, StorageError> {
        let taxonomy = memory_default_taxonomy();
        let category = normalize_label_name(&classification.category);
        let category = taxonomy
            .categories
            .into_iter()
            .find(|candidate| candidate.name == category)
            .unwrap_or_else(|| memory_category(0, "other", "Fallback category", "seed"));
        let topics = if classification.topics.is_empty() {
            vec!["general".to_string()]
        } else {
            classification
                .topics
                .iter()
                .map(|topic| normalize_label_name(topic))
                .collect::<Vec<_>>()
        };
        Ok(ResolvedEmailClassification {
            category_id: category.id,
            category: category.name,
            topic_ids: vec![],
            topics,
            reason: classification.reason.clone(),
            confidence: classification.confidence.min(100),
        })
    }

    async fn find_matching_email_rule(
        &self,
        mailbox_id: &str,
        classification: &ResolvedEmailClassification,
    ) -> Result<Option<EmailRule>, StorageError> {
        let _ = (mailbox_id, classification);
        Ok(None)
    }

    async fn record_email_classification(
        &self,
        key: &DedupeKey,
        classification: &ResolvedEmailClassification,
        decision_source: &str,
        matched_rule: Option<&EmailRule>,
    ) -> Result<(), StorageError> {
        let _ = (key, classification, decision_source, matched_rule);
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct MemoryProcessingStore {
    statuses: Arc<Mutex<HashMap<DedupeKey, String>>>,
    safety_results: Arc<Mutex<HashMap<DedupeKey, StoredSafetyResult>>>,
    sender_reviews: Arc<Mutex<HashMap<String, SenderReviewRecord>>>,
    agent_decisions: Arc<Mutex<HashMap<DedupeKey, StoredAgentDecision>>>,
    outbound_actions: Arc<Mutex<HashMap<DedupeKey, StoredOutboundAction>>>,
    outbound_reviews: Arc<Mutex<HashMap<DedupeKey, StoredOutboundReview>>>,
    classification: Arc<Mutex<MemoryClassificationState>>,
}

#[derive(Debug, Clone)]
struct MemoryClassificationState {
    categories: Vec<EmailCategory>,
    topics: Vec<EmailTopic>,
    rules: Vec<EmailRule>,
    next_category_id: i64,
    next_topic_id: i64,
    next_rule_id: i64,
    records: HashMap<DedupeKey, StoredEmailClassification>,
}

impl Default for MemoryClassificationState {
    fn default() -> Self {
        let categories = default_categories()
            .into_iter()
            .enumerate()
            .map(|(index, (name, description))| {
                memory_category((index + 1) as i64, name, description, "seed")
            })
            .collect::<Vec<_>>();
        let topics = default_topics()
            .into_iter()
            .enumerate()
            .map(|(index, (name, description))| {
                memory_topic((index + 1) as i64, name, description, "seed")
            })
            .collect::<Vec<_>>();
        Self {
            next_category_id: categories.len() as i64 + 1,
            next_topic_id: topics.len() as i64 + 1,
            categories,
            topics,
            rules: vec![],
            next_rule_id: 1,
            records: HashMap::new(),
        }
    }
}

fn memory_category(id: i64, name: &str, description: &str, source: &str) -> EmailCategory {
    EmailCategory {
        id,
        name: name.to_string(),
        description: description.to_string(),
        status: "active".to_string(),
        source: source.to_string(),
        created_at: "memory".to_string(),
        updated_at: "memory".to_string(),
    }
}

fn memory_topic(id: i64, name: &str, description: &str, source: &str) -> EmailTopic {
    EmailTopic {
        id,
        name: name.to_string(),
        description: description.to_string(),
        status: "active".to_string(),
        source: source.to_string(),
        created_at: "memory".to_string(),
        updated_at: "memory".to_string(),
    }
}

fn memory_default_taxonomy() -> EmailTaxonomy {
    let state = MemoryClassificationState::default();
    EmailTaxonomy {
        categories: state.categories,
        topics: state.topics,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSafetyResult {
    pub category: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderReviewRecord {
    pub mailbox_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAgentDecision {
    pub action: String,
    pub safety_notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOutboundAction {
    pub kind: String,
    pub recipients: Vec<String>,
    pub subject: String,
    pub body: Option<String>,
    pub body_redacted: bool,
    pub reason: String,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOutboundReview {
    pub status: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEmailClassification {
    pub category: String,
    pub topics: Vec<String>,
    pub reason: String,
    pub confidence: u8,
    pub decision_source: String,
    pub matched_rule_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProcessedEmail {
    pub run_id: String,
    pub mailbox_id: String,
    pub uid_validity: u64,
    pub uid: u64,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from_addr: String,
    pub subject: String,
    pub inbound_body: Option<String>,
    pub inbound_body_truncated: bool,
    pub status: String,
    pub safety_category: Option<String>,
    pub safety_reason: Option<String>,
    pub agent_action: Option<String>,
    pub agent_safety_notes: Option<String>,
    pub outbound_action: Option<String>,
    pub outbound_recipients: Vec<String>,
    pub outbound_subject: Option<String>,
    pub outbound_body: Option<String>,
    pub outbound_body_redacted: bool,
    pub outbound_message_id: Option<String>,
    pub outbound_reason: Option<String>,
    pub outbound_review_status: Option<String>,
    pub outbound_review_reason: Option<String>,
    pub classification_category: Option<String>,
    pub classification_topics: Vec<String>,
    pub classification_reason: Option<String>,
    pub classification_confidence: Option<u16>,
    pub decision_source: Option<String>,
    pub matched_rule_id: Option<i64>,
    pub matched_rule_name: Option<String>,
    pub matched_rule_goal: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub logs: Vec<ProcessedEmailLog>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProcessedEmailLog {
    pub level: String,
    pub run_id: String,
    pub action: String,
    pub status: String,
    pub duration_ms: u64,
    pub detail: Option<String>,
    pub created_at: String,
}

impl MemoryProcessingStore {
    pub fn status(&self, key: &DedupeKey) -> Option<String> {
        self.statuses
            .lock()
            .expect("memory processing store poisoned")
            .get(key)
            .cloned()
    }

    pub fn safety_result(&self, key: &DedupeKey) -> Option<StoredSafetyResult> {
        self.safety_results
            .lock()
            .expect("memory safety result store poisoned")
            .get(key)
            .cloned()
    }

    pub fn sender_review(&self, sender: &str) -> Option<SenderReviewRecord> {
        self.sender_reviews
            .lock()
            .expect("memory sender review store poisoned")
            .get(sender)
            .cloned()
    }

    pub fn agent_decision(&self, key: &DedupeKey) -> Option<StoredAgentDecision> {
        self.agent_decisions
            .lock()
            .expect("memory agent decision store poisoned")
            .get(key)
            .cloned()
    }

    pub fn outbound_action(&self, key: &DedupeKey) -> Option<StoredOutboundAction> {
        self.outbound_actions
            .lock()
            .expect("memory outbound action store poisoned")
            .get(key)
            .cloned()
    }

    pub fn outbound_review(&self, key: &DedupeKey) -> Option<StoredOutboundReview> {
        self.outbound_reviews
            .lock()
            .expect("memory outbound review store poisoned")
            .get(key)
            .cloned()
    }

    pub fn email_classification(&self, key: &DedupeKey) -> Option<StoredEmailClassification> {
        self.classification
            .lock()
            .expect("memory classification store poisoned")
            .records
            .get(key)
            .cloned()
    }

    pub fn add_email_rule(&self, mut rule: NewEmailRule) -> EmailRule {
        let mut state = self
            .classification
            .lock()
            .expect("memory classification store poisoned");
        rule.name = rule.name.trim().to_string();
        let id = state.next_rule_id;
        state.next_rule_id += 1;
        let category = state
            .categories
            .iter()
            .find(|category| category.id == rule.category_id)
            .map(|category| category.name.clone())
            .unwrap_or_else(|| "other".to_string());
        let topics = rule
            .topic_ids
            .iter()
            .filter_map(|id| {
                state
                    .topics
                    .iter()
                    .find(|topic| topic.id == *id)
                    .map(|topic| topic.name.clone())
            })
            .collect::<Vec<_>>();
        let stored = EmailRule {
            id,
            mailbox_id: rule.mailbox_id,
            name: rule.name,
            category_id: rule.category_id,
            category,
            topic_ids: rule.topic_ids,
            topics,
            action: rule.action,
            reply_goal: rule.reply_goal,
            enabled: rule.enabled,
            priority: rule.priority,
            created_at: "memory".to_string(),
            updated_at: "memory".to_string(),
        };
        state.rules.push(stored.clone());
        stored
    }
}

pub(crate) fn parse_run_id(run_id: &str) -> Result<uuid::Uuid, StorageError> {
    Ok(uuid::Uuid::parse_str(run_id)?)
}

pub(crate) fn validate_applied_migration(
    migration: &Migration,
    expected_checksum: &str,
    applied: &AppliedMigration,
) -> Result<(), StorageError> {
    if applied.name != migration.name {
        return Err(StorageError::MigrationNameMismatch {
            version: migration.version,
            expected_name: migration.name,
            applied_name: applied.name.clone(),
        });
    }
    if applied.checksum != expected_checksum {
        return Err(StorageError::MigrationChecksumMismatch {
            version: migration.version,
            expected_checksum: expected_checksum.to_string(),
            applied_checksum: applied.checksum.clone(),
        });
    }
    Ok(())
}

pub(crate) fn migration_checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.as_bytes()))
}

#[async_trait::async_trait]
impl ProcessingStore for MemoryProcessingStore {
    async fn claim_message(
        &self,
        _run_id: &str,
        message: &InboundMessage,
    ) -> Result<ProcessingClaim, StorageError> {
        let mut statuses = self
            .statuses
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let key = message.metadata.dedupe_key();
        match statuses.get(&key) {
            None => {
                statuses.insert(key, PROCESSING_STATUS_PROCESSING.to_string());
                Ok(ProcessingClaim::Claimed)
            }
            Some(status) if processing_status_can_reclaim(status, false) => {
                statuses.insert(key, PROCESSING_STATUS_PROCESSING.to_string());
                Ok(ProcessingClaim::Claimed)
            }
            Some(status) => Ok(processing_claim_for_existing_status(status)),
        }
    }

    async fn update_message_status(
        &self,
        key: &DedupeKey,
        status: &str,
        _outbound_action: Option<&OutboundActionKind>,
    ) -> Result<(), StorageError> {
        self.statuses
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .insert(key.clone(), status.to_string());
        Ok(())
    }

    async fn record_safety_result(
        &self,
        key: &DedupeKey,
        category: &SafetyCategory,
        reason: &str,
    ) -> Result<(), StorageError> {
        self.safety_results
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .insert(
                key.clone(),
                StoredSafetyResult {
                    category: safety_category_value(category).to_string(),
                    reason: reason.to_string(),
                },
            );
        Ok(())
    }

    async fn record_agent_decision(
        &self,
        key: &DedupeKey,
        decision: &AgentDecision,
    ) -> Result<(), StorageError> {
        self.agent_decisions
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .insert(
                key.clone(),
                StoredAgentDecision {
                    action: outbound_action_value(&decision.action.kind).to_string(),
                    safety_notes: decision.safety_notes.clone(),
                },
            );
        Ok(())
    }

    async fn record_outbound_action(
        &self,
        key: &DedupeKey,
        action: &OutboundAction,
    ) -> Result<(), StorageError> {
        let (body, body_redacted) = outbound_body_for_storage(action);
        self.outbound_actions
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .insert(
                key.clone(),
                StoredOutboundAction {
                    kind: outbound_action_value(&action.kind).to_string(),
                    recipients: action.recipients.clone(),
                    subject: action.subject.clone(),
                    body: body.map(ToString::to_string),
                    body_redacted,
                    reason: action.reason.clone(),
                    message_id: action.message_id.clone(),
                },
            );
        Ok(())
    }

    async fn record_outbound_review(
        &self,
        key: &DedupeKey,
        status: &str,
        reason: &str,
    ) -> Result<(), StorageError> {
        self.outbound_reviews
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .insert(
                key.clone(),
                StoredOutboundReview {
                    status: status.to_string(),
                    reason: reason.to_string(),
                },
            );
        Ok(())
    }

    async fn ensure_default_classification_policy(
        &self,
        config: &AppConfig,
    ) -> Result<(), StorageError> {
        let mut state = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let Some(category) = state
            .categories
            .iter()
            .find(|category| category.name == "marketing_vendor")
            .cloned()
        else {
            return Ok(());
        };
        for mailbox in &config.mailboxes {
            if state
                .rules
                .iter()
                .any(|rule| rule.mailbox_id == mailbox.id && rule.category_id == category.id)
            {
                continue;
            }
            let id = state.next_rule_id;
            state.next_rule_id += 1;
            state.rules.push(EmailRule {
                id,
                mailbox_id: mailbox.id.clone(),
                name: "Auto-decline marketing/vendor outreach".to_string(),
                category_id: category.id,
                category: category.name.clone(),
                topic_ids: vec![],
                topics: vec![],
                action: EmailRuleAction::Reply,
                reply_goal: DEFAULT_MARKETING_REPLY_GOAL.to_string(),
                enabled: true,
                priority: 100,
                created_at: "memory".to_string(),
                updated_at: "memory".to_string(),
            });
        }
        Ok(())
    }

    async fn active_email_taxonomy(&self) -> Result<EmailTaxonomy, StorageError> {
        let state = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        Ok(EmailTaxonomy {
            categories: state
                .categories
                .iter()
                .filter(|category| category.status == "active")
                .cloned()
                .collect(),
            topics: state
                .topics
                .iter()
                .filter(|topic| topic.status == "active")
                .cloned()
                .collect(),
        })
    }

    async fn resolve_email_classification(
        &self,
        classification: &EmailClassification,
    ) -> Result<ResolvedEmailClassification, StorageError> {
        let mut state = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let category_name = normalize_label_name(&classification.category);
        let category = match state
            .categories
            .iter()
            .find(|category| category.name == category_name)
            .cloned()
        {
            Some(category) => category,
            None => {
                let category = memory_category(
                    state.next_category_id,
                    &category_name,
                    "AI-created category",
                    "ai",
                );
                state.next_category_id += 1;
                state.categories.push(category.clone());
                category
            }
        };

        let mut topic_ids = Vec::new();
        let mut topics = Vec::new();
        let topic_names = if classification.topics.is_empty() {
            vec!["general".to_string()]
        } else {
            classification
                .topics
                .iter()
                .map(|topic| normalize_label_name(topic))
                .collect::<Vec<_>>()
        };
        for topic_name in topic_names {
            let topic = match state
                .topics
                .iter()
                .find(|topic| topic.name == topic_name)
                .cloned()
            {
                Some(topic) => topic,
                None => {
                    let topic =
                        memory_topic(state.next_topic_id, &topic_name, "AI-created topic", "ai");
                    state.next_topic_id += 1;
                    state.topics.push(topic.clone());
                    topic
                }
            };
            if !topic_ids.contains(&topic.id) {
                topic_ids.push(topic.id);
                topics.push(topic.name);
            }
        }

        Ok(ResolvedEmailClassification {
            category_id: category.id,
            category: category.name,
            topic_ids,
            topics,
            reason: classification.reason.clone(),
            confidence: classification.confidence.min(100),
        })
    }

    async fn find_matching_email_rule(
        &self,
        mailbox_id: &str,
        classification: &ResolvedEmailClassification,
    ) -> Result<Option<EmailRule>, StorageError> {
        let mut rules = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .rules
            .iter()
            .filter(|rule| {
                rule.enabled
                    && rule.mailbox_id == mailbox_id
                    && rule.category_id == classification.category_id
                    && (rule.topic_ids.is_empty()
                        || rule
                            .topic_ids
                            .iter()
                            .any(|topic_id| classification.topic_ids.contains(topic_id)))
            })
            .cloned()
            .collect::<Vec<_>>();
        rules.sort_by_key(|rule| {
            (
                if rule.topic_ids.is_empty() { 1 } else { 0 },
                rule.priority,
                rule.id,
            )
        });
        Ok(rules.into_iter().next())
    }

    async fn record_email_classification(
        &self,
        key: &DedupeKey,
        classification: &ResolvedEmailClassification,
        decision_source: &str,
        matched_rule: Option<&EmailRule>,
    ) -> Result<(), StorageError> {
        self.classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .records
            .insert(
                key.clone(),
                StoredEmailClassification {
                    category: classification.category.clone(),
                    topics: classification.topics.clone(),
                    reason: classification.reason.clone(),
                    confidence: classification.confidence,
                    decision_source: decision_source.to_string(),
                    matched_rule_name: matched_rule.map(|rule| rule.name.clone()),
                },
            );
        Ok(())
    }

    async fn upsert_sender_review(
        &self,
        sender: &str,
        mailbox_id: &str,
        reason: &str,
    ) -> Result<(), StorageError> {
        self.sender_reviews
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .insert(
                sender.to_string(),
                SenderReviewRecord {
                    mailbox_id: mailbox_id.to_string(),
                    reason: reason.to_string(),
                },
            );
        Ok(())
    }
}

pub fn processing_status_can_reclaim(status: &str, stale: bool) -> bool {
    status == PROCESSING_STATUS_SEND_FAILED
        || status == PROCESSING_STATUS_RETRYABLE_FAILED
        || (status == PROCESSING_STATUS_PROCESSING && stale)
}

pub fn processing_claim_for_existing_status(status: &str) -> ProcessingClaim {
    if status == PROCESSING_STATUS_PROCESSING {
        ProcessingClaim::InProgress {
            status: status.to_string(),
        }
    } else {
        ProcessingClaim::AlreadyFinished {
            status: status.to_string(),
        }
    }
}

pub fn outbound_action_value(kind: &OutboundActionKind) -> &'static str {
    match kind {
        OutboundActionKind::Reply => "reply",
        OutboundActionKind::Forward => "forward",
        OutboundActionKind::Noop => "noop",
    }
}

pub fn safety_category_value(category: &SafetyCategory) -> &'static str {
    match category {
        SafetyCategory::Safe => "safe",
        SafetyCategory::Jailbreak => "jailbreak",
        SafetyCategory::PromptInjection => "prompt_injection",
        SafetyCategory::Hacking => "hacking",
        SafetyCategory::SensitiveExfiltration => "sensitive_exfiltration",
        SafetyCategory::Unknown => "unknown",
    }
}

pub(crate) fn outbound_body_for_storage(action: &OutboundAction) -> (Option<&str>, bool) {
    match action.kind {
        OutboundActionKind::Reply => (Some(action.body.as_str()), false),
        OutboundActionKind::Forward => (None, true),
        OutboundActionKind::Noop => (None, false),
    }
}

pub(crate) fn inbound_body_for_storage(message: &InboundMessage) -> (String, bool) {
    let mut output = String::new();
    let mut truncated = false;
    for (index, character) in message.plain_text.chars().enumerate() {
        if index >= INBOUND_BODY_STORAGE_MAX_CHARS {
            truncated = true;
            break;
        }
        output.push(character);
    }
    (output, truncated)
}

pub(crate) fn empty_string_as_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn metadata_only_schema_guard(sql: &str) -> Result<(), String> {
    let lowered = sql.to_ascii_lowercase();
    let forbidden = [
        " raw_email",
        "email_body",
        " body text",
        " body bytea",
        " parsed_content",
        " message_content",
    ];
    for token in forbidden {
        if lowered.contains(token) {
            return Err(format!(
                "migration contains forbidden email-content column token: {token}"
            ));
        }
    }
    Ok(())
}

pub fn retention_delete_sql(retention_days: u16) -> String {
    format!(
        "DELETE FROM action_logs WHERE created_at < now() - interval '{} days'",
        retention_days
    )
}

pub(crate) fn log_level_value(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
        LogLevel::Fatal => "fatal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AgentConfig, AiConfig, AiProtocol, DatabaseConfig, ImapConfig, LoggingConfig,
        MailboxConfig, PromptConfig, ReviewConfig, SmtpConfig,
    };
    use crate::mail::{InboundMessage, MessageMetadata};

    #[test]
    fn migration_is_metadata_only() {
        metadata_only_schema_guard(INIT_SQL).unwrap();
    }

    #[test]
    fn metadata_guard_rejects_content_columns() {
        let error = metadata_only_schema_guard("CREATE TABLE t (email_body TEXT)")
            .unwrap_err()
            .to_string();
        assert!(error.contains("forbidden email-content"));
    }

    #[test]
    fn migration_defines_expected_tables() {
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS processing_runs"));
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS action_logs"));
        assert!(INIT_SQL.contains("message_uid_validity BIGINT"));
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS banned_senders"));
        assert!(INIT_SQL.contains("ADD COLUMN IF NOT EXISTS outbound_body"));
        assert!(INIT_SQL.contains("ADD COLUMN IF NOT EXISTS agent_safety_notes"));
        assert!(HISTORY_BODY_THREADING_SQL.contains("ADD COLUMN IF NOT EXISTS inbound_body"));
        assert!(HISTORY_BODY_THREADING_SQL.contains("ADD COLUMN IF NOT EXISTS thread_id"));
        assert!(HISTORY_BODY_THREADING_SQL.contains("ADD COLUMN IF NOT EXISTS outbound_message_id"));
        assert!(DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL
            .contains("email_rules_default_marketing_seed_unique_idx"));
    }

    #[test]
    fn migration_runner_defines_version_tracking() {
        assert!(SCHEMA_MIGRATIONS_SQL.contains("CREATE TABLE IF NOT EXISTS schema_migrations"));
        assert!(SCHEMA_MIGRATIONS_SQL.contains("version INTEGER PRIMARY KEY"));
        assert!(SCHEMA_MIGRATIONS_SQL.contains("checksum TEXT NOT NULL"));
        assert_eq!(MIGRATIONS[0].version, 1);
        assert_eq!(MIGRATIONS[0].name, "001_init");
        assert_eq!(MIGRATIONS[0].sql, INIT_SQL);
        assert_eq!(MIGRATIONS[1].version, 2);
        assert_eq!(MIGRATIONS[1].name, "002_history_body_threading");
        assert_eq!(MIGRATIONS[1].sql, HISTORY_BODY_THREADING_SQL);
        assert_eq!(MIGRATIONS[2].version, 3);
        assert_eq!(MIGRATIONS[2].name, "003_email_classification_rules");
        assert_eq!(MIGRATIONS[2].sql, EMAIL_CLASSIFICATION_RULES_SQL);
        assert_eq!(MIGRATIONS[3].version, 4);
        assert_eq!(MIGRATIONS[3].name, "004_default_email_rule_seed_uniqueness");
        assert_eq!(MIGRATIONS[3].sql, DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL);
    }

    #[test]
    fn migration_versions_are_strictly_increasing() {
        for pair in MIGRATIONS.windows(2) {
            assert!(
                pair[0].version < pair[1].version,
                "migration versions must be strictly increasing"
            );
        }
    }

    #[test]
    fn migration_checksum_is_stable_sha256_hex() {
        let checksum = migration_checksum("SELECT 1;");
        assert_eq!(checksum.len(), 64);
        assert_eq!(checksum, migration_checksum("SELECT 1;"));
        assert_ne!(checksum, migration_checksum("SELECT 2;"));
    }

    #[test]
    fn applied_migration_validation_rejects_name_or_checksum_drift() {
        let migration = Migration {
            version: 7,
            name: "007_test",
            sql: "SELECT 1;",
        };
        let checksum = migration_checksum(migration.sql);
        validate_applied_migration(
            &migration,
            &checksum,
            &AppliedMigration {
                name: migration.name.to_string(),
                checksum: checksum.clone(),
            },
        )
        .unwrap();

        let name_error = validate_applied_migration(
            &migration,
            &checksum,
            &AppliedMigration {
                name: "007_other".to_string(),
                checksum: checksum.clone(),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(name_error.contains("expected \"007_test\""));

        let checksum_error = validate_applied_migration(
            &migration,
            &checksum,
            &AppliedMigration {
                name: migration.name.to_string(),
                checksum: "different".to_string(),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(checksum_error.contains("checksum mismatch"));
    }

    #[test]
    fn retention_sql_uses_configured_days() {
        assert_eq!(
            retention_delete_sql(180),
            "DELETE FROM action_logs WHERE created_at < now() - interval '180 days'"
        );
    }

    #[test]
    fn level_values_match_storage_check_constraint() {
        assert_eq!(log_level_value(LogLevel::Fatal), "fatal");
        assert_eq!(log_level_value(LogLevel::Debug), "debug");
        assert_eq!(log_level_value(LogLevel::Info), "info");
        assert_eq!(log_level_value(LogLevel::Warn), "warn");
        assert_eq!(log_level_value(LogLevel::Error), "error");
    }

    #[test]
    fn processing_status_reclaim_rules_retry_failed_and_stale_processing() {
        assert!(processing_status_can_reclaim(
            PROCESSING_STATUS_SEND_FAILED,
            false
        ));
        assert!(processing_status_can_reclaim(
            PROCESSING_STATUS_RETRYABLE_FAILED,
            false
        ));
        assert!(processing_status_can_reclaim(
            PROCESSING_STATUS_PROCESSING,
            true
        ));
        assert!(!processing_status_can_reclaim(
            PROCESSING_STATUS_PROCESSING,
            false
        ));
        assert!(!processing_status_can_reclaim("replied", true));
    }

    #[test]
    fn processing_claim_classifies_existing_status() {
        assert_eq!(
            processing_claim_for_existing_status(PROCESSING_STATUS_PROCESSING),
            ProcessingClaim::InProgress {
                status: PROCESSING_STATUS_PROCESSING.to_string()
            }
        );
        assert_eq!(
            processing_claim_for_existing_status("replied"),
            ProcessingClaim::AlreadyFinished {
                status: "replied".to_string()
            }
        );
    }

    #[test]
    fn outbound_action_values_match_storage_terms() {
        assert_eq!(outbound_action_value(&OutboundActionKind::Reply), "reply");
        assert_eq!(
            outbound_action_value(&OutboundActionKind::Forward),
            "forward"
        );
        assert_eq!(outbound_action_value(&OutboundActionKind::Noop), "noop");
    }

    #[test]
    fn outbound_body_storage_keeps_replies_and_redacts_forwards() {
        let reply = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            reason: "known answer".to_string(),
            message_id: None,
            in_reply_to: None,
            references: vec![],
        };
        assert_eq!(outbound_body_for_storage(&reply), (Some("Answer"), false));

        let forward = OutboundAction {
            kind: OutboundActionKind::Forward,
            recipients: vec!["human@example.com".to_string()],
            subject: "Fwd: Question".to_string(),
            body: "contains original inbound body".to_string(),
            reason: "human review".to_string(),
            message_id: None,
            in_reply_to: None,
            references: vec![],
        };
        assert_eq!(outbound_body_for_storage(&forward), (None, true));
    }

    #[test]
    fn inbound_body_storage_caps_large_message_bodies() {
        let mut message = message(7);
        message.plain_text = "a".repeat(INBOUND_BODY_STORAGE_MAX_CHARS + 10);

        let (body, truncated) = inbound_body_for_storage(&message);

        assert_eq!(body.len(), INBOUND_BODY_STORAGE_MAX_CHARS);
        assert!(truncated);
    }

    #[test]
    fn safety_category_values_match_storage_terms() {
        assert_eq!(
            safety_category_value(&SafetyCategory::PromptInjection),
            "prompt_injection"
        );
        assert_eq!(
            safety_category_value(&SafetyCategory::SensitiveExfiltration),
            "sensitive_exfiltration"
        );
        assert_eq!(safety_category_value(&SafetyCategory::Safe), "safe");
    }

    #[test]
    fn postgres_uuid_params_accept_uuid_values() {
        fn assert_postgres_param<T: tokio_postgres::types::ToSql + Sync>() {}

        assert_postgres_param::<uuid::Uuid>();
    }

    #[test]
    fn parse_run_id_rejects_non_uuid_values() {
        let error = parse_run_id("not-a-uuid").unwrap_err().to_string();
        assert!(error.contains("processing run id is not a uuid"));
    }

    #[tokio::test]
    async fn memory_processing_store_claims_updates_and_skips_finished_messages() {
        let store = MemoryProcessingStore::default();
        let message = message(42);
        let key = message.metadata.dedupe_key();

        assert_eq!(
            store.claim_message("run-test", &message).await.unwrap(),
            ProcessingClaim::Claimed
        );
        assert_eq!(
            store.status(&key),
            Some(PROCESSING_STATUS_PROCESSING.to_string())
        );

        store
            .update_message_status(&key, "replied", Some(&OutboundActionKind::Reply))
            .await
            .unwrap();
        assert_eq!(
            store.claim_message("run-test", &message).await.unwrap(),
            ProcessingClaim::AlreadyFinished {
                status: "replied".to_string()
            }
        );
    }

    #[tokio::test]
    async fn memory_processing_store_reclaims_retryable_failures() {
        let store = MemoryProcessingStore::default();
        for (uid, status) in [
            (43, PROCESSING_STATUS_SEND_FAILED),
            (44, PROCESSING_STATUS_RETRYABLE_FAILED),
        ] {
            let message = message(uid);
            let key = message.metadata.dedupe_key();
            store.claim_message("run-test", &message).await.unwrap();
            store
                .update_message_status(&key, status, None)
                .await
                .unwrap();
            assert_eq!(
                store.claim_message("run-test-2", &message).await.unwrap(),
                ProcessingClaim::Claimed
            );
            assert_eq!(
                store.status(&key),
                Some(PROCESSING_STATUS_PROCESSING.to_string())
            );
        }
    }

    #[tokio::test]
    async fn memory_processing_store_records_safety_and_sender_review_state() {
        let store = MemoryProcessingStore::default();
        let message = message(44);
        let key = message.metadata.dedupe_key();

        store
            .record_safety_result(
                &key,
                &SafetyCategory::PromptInjection,
                "tries to override policy",
            )
            .await
            .unwrap();
        store
            .upsert_sender_review(
                &message.metadata.from_addr,
                &message.metadata.mailbox_id,
                "tries to override policy",
            )
            .await
            .unwrap();

        assert_eq!(
            store.safety_result(&key),
            Some(StoredSafetyResult {
                category: "prompt_injection".to_string(),
                reason: "tries to override policy".to_string()
            })
        );
        assert_eq!(
            store.sender_review("person@example.com"),
            Some(SenderReviewRecord {
                mailbox_id: "support".to_string(),
                reason: "tries to override policy".to_string()
            })
        );
    }

    #[tokio::test]
    async fn memory_processing_store_records_history_outcomes() {
        let store = MemoryProcessingStore::default();
        let message = message(45);
        let key = message.metadata.dedupe_key();
        let action = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            reason: "known answer".to_string(),
            message_id: Some("<reply@example.com>".to_string()),
            in_reply_to: Some("<inbound@example.com>".to_string()),
            references: vec!["<root@example.com>".to_string()],
        };
        let decision = AgentDecision {
            action: action.clone(),
            safety_notes: "safe".to_string(),
        };

        store.record_agent_decision(&key, &decision).await.unwrap();
        store.record_outbound_action(&key, &action).await.unwrap();
        store
            .record_outbound_review(&key, "approved", "looks safe")
            .await
            .unwrap();

        assert_eq!(
            store.agent_decision(&key),
            Some(StoredAgentDecision {
                action: "reply".to_string(),
                safety_notes: "safe".to_string()
            })
        );
        assert_eq!(
            store.outbound_action(&key),
            Some(StoredOutboundAction {
                kind: "reply".to_string(),
                recipients: vec!["person@example.com".to_string()],
                subject: "Re: Question".to_string(),
                body: Some("Answer".to_string()),
                body_redacted: false,
                reason: "known answer".to_string(),
                message_id: Some("<reply@example.com>".to_string())
            })
        );
        assert_eq!(
            store.outbound_review(&key),
            Some(StoredOutboundReview {
                status: "approved".to_string(),
                reason: "looks safe".to_string()
            })
        );
    }

    #[tokio::test]
    async fn memory_store_resolves_unknown_labels_and_records_classification() {
        let store = MemoryProcessingStore::default();
        let message = message(46);
        let key = message.metadata.dedupe_key();

        let resolved = store
            .resolve_email_classification(&EmailClassification {
                category: "New Category!".to_string(),
                topics: vec![
                    "Dense Mem".to_string(),
                    "Dense Mem".to_string(),
                    "New Topic".to_string(),
                ],
                reason: "category and topic are model-created".to_string(),
                confidence: 255,
            })
            .await
            .unwrap();

        assert_eq!(resolved.category, "new_category");
        assert_eq!(resolved.topics, vec!["dense_mem", "new_topic"]);
        assert_eq!(resolved.confidence, 100);

        let taxonomy = store.active_email_taxonomy().await.unwrap();
        assert!(taxonomy
            .categories
            .iter()
            .any(|category| category.name == "new_category" && category.source == "ai"));
        assert!(taxonomy
            .topics
            .iter()
            .any(|topic| topic.name == "new_topic" && topic.source == "ai"));

        store
            .record_email_classification(&key, &resolved, "agent", None)
            .await
            .unwrap();

        assert_eq!(
            store.email_classification(&key),
            Some(StoredEmailClassification {
                category: "new_category".to_string(),
                topics: vec!["dense_mem".to_string(), "new_topic".to_string()],
                reason: "category and topic are model-created".to_string(),
                confidence: 100,
                decision_source: "agent".to_string(),
                matched_rule_name: None,
            })
        );
    }

    #[tokio::test]
    async fn memory_store_defaults_empty_topics_to_general() {
        let store = MemoryProcessingStore::default();

        let resolved = store
            .resolve_email_classification(&EmailClassification {
                category: "question".to_string(),
                topics: vec![],
                reason: "asks about setup".to_string(),
                confidence: 87,
            })
            .await
            .unwrap();

        assert_eq!(resolved.category, "question");
        assert_eq!(resolved.topics, vec!["general"]);
        assert_eq!(resolved.confidence, 87);
    }

    #[tokio::test]
    async fn memory_store_matches_topic_specific_rule_before_general_rule() {
        let store = MemoryProcessingStore::default();
        let taxonomy = store.active_email_taxonomy().await.unwrap();
        let question = taxonomy
            .categories
            .iter()
            .find(|category| category.name == "question")
            .unwrap();
        let dense_mem = taxonomy
            .topics
            .iter()
            .find(|topic| topic.name == "dense_mem")
            .unwrap();

        let general = store.add_email_rule(NewEmailRule {
            mailbox_id: "support".to_string(),
            name: "General question rule".to_string(),
            category_id: question.id,
            topic_ids: vec![],
            action: EmailRuleAction::Forward,
            reply_goal: "Forward broad questions to Mark.".to_string(),
            enabled: true,
            priority: 1,
        });
        let topic_specific = store.add_email_rule(NewEmailRule {
            mailbox_id: "support".to_string(),
            name: "Dense-Mem answer rule".to_string(),
            category_id: question.id,
            topic_ids: vec![dense_mem.id],
            action: EmailRuleAction::Reply,
            reply_goal: "Answer Dense-Mem setup questions.".to_string(),
            enabled: true,
            priority: 100,
        });
        store.add_email_rule(NewEmailRule {
            mailbox_id: "support".to_string(),
            name: "Disabled Dense-Mem rule".to_string(),
            category_id: question.id,
            topic_ids: vec![dense_mem.id],
            action: EmailRuleAction::Noop,
            reply_goal: String::new(),
            enabled: false,
            priority: 0,
        });

        let resolved = store
            .resolve_email_classification(&EmailClassification {
                category: "question".to_string(),
                topics: vec!["dense_mem".to_string()],
                reason: "asks about Dense-Mem".to_string(),
                confidence: 91,
            })
            .await
            .unwrap();

        assert_eq!(
            store
                .find_matching_email_rule("support", &resolved)
                .await
                .unwrap(),
            Some(topic_specific.clone())
        );
        assert_eq!(
            store
                .find_matching_email_rule("other", &resolved)
                .await
                .unwrap(),
            None
        );
        assert_ne!(general.id, topic_specific.id);
    }

    #[tokio::test]
    async fn memory_store_seeds_default_marketing_rule_once_per_mailbox() {
        let store = MemoryProcessingStore::default();
        let config = app_config_with_mailboxes(vec!["support", "sales"]);

        store
            .ensure_default_classification_policy(&config)
            .await
            .unwrap();
        store
            .ensure_default_classification_policy(&config)
            .await
            .unwrap();

        let resolved = store
            .resolve_email_classification(&EmailClassification {
                category: "marketing_vendor".to_string(),
                topics: vec!["general".to_string()],
                reason: "offers paid ads".to_string(),
                confidence: 94,
            })
            .await
            .unwrap();

        let support_rule = store
            .find_matching_email_rule("support", &resolved)
            .await
            .unwrap()
            .unwrap();
        let sales_rule = store
            .find_matching_email_rule("sales", &resolved)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(support_rule.name, "Auto-decline marketing/vendor outreach");
        assert_eq!(support_rule.reply_goal, DEFAULT_MARKETING_REPLY_GOAL);
        assert_eq!(sales_rule.name, "Auto-decline marketing/vendor outreach");
        assert_ne!(support_rule.id, sales_rule.id);
    }

    struct MinimalStore;

    #[async_trait::async_trait]
    impl ProcessingStore for MinimalStore {
        async fn claim_message(
            &self,
            _run_id: &str,
            _message: &InboundMessage,
        ) -> Result<ProcessingClaim, StorageError> {
            Ok(ProcessingClaim::Claimed)
        }

        async fn update_message_status(
            &self,
            _key: &DedupeKey,
            _status: &str,
            _outbound_action: Option<&OutboundActionKind>,
        ) -> Result<(), StorageError> {
            Ok(())
        }

        async fn record_safety_result(
            &self,
            _key: &DedupeKey,
            _category: &SafetyCategory,
            _reason: &str,
        ) -> Result<(), StorageError> {
            Ok(())
        }

        async fn upsert_sender_review(
            &self,
            _sender: &str,
            _mailbox_id: &str,
            _reason: &str,
        ) -> Result<(), StorageError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn processing_store_trait_defaults_are_noops() {
        let store = MinimalStore;
        let message = message(47);
        let key = message.metadata.dedupe_key();
        let action = OutboundAction {
            kind: OutboundActionKind::Noop,
            recipients: vec![],
            subject: String::new(),
            body: String::new(),
            reason: "nothing to do".to_string(),
            message_id: None,
            in_reply_to: None,
            references: vec![],
        };
        let decision = AgentDecision {
            action,
            safety_notes: "safe".to_string(),
        };

        store.record_agent_decision(&key, &decision).await.unwrap();
        store
            .record_outbound_action(&key, &decision.action)
            .await
            .unwrap();
        store
            .record_outbound_review(&key, "approved", "safe")
            .await
            .unwrap();
        store
            .ensure_default_classification_policy(&app_config_with_mailboxes(vec!["support"]))
            .await
            .unwrap();

        let taxonomy = store.active_email_taxonomy().await.unwrap();
        assert!(taxonomy
            .categories
            .iter()
            .any(|category| category.name == "question"));

        let resolved = store
            .resolve_email_classification(&EmailClassification {
                category: "question".to_string(),
                topics: vec!["general".to_string()],
                reason: "default implementation".to_string(),
                confidence: 99,
            })
            .await
            .unwrap();
        assert_eq!(resolved.category, "question");
        assert_eq!(
            store
                .find_matching_email_rule("support", &resolved)
                .await
                .unwrap(),
            None
        );
        store
            .record_email_classification(&key, &resolved, "agent", None)
            .await
            .unwrap();
    }

    fn message(uid: u64) -> InboundMessage {
        InboundMessage {
            metadata: MessageMetadata {
                mailbox_id: "support".to_string(),
                uid_validity: 1,
                uid,
                message_id: Some(format!("<{uid}@example.com>")),
                in_reply_to: None,
                references: vec![],
                from_addr: "person@example.com".to_string(),
                subject: "Question".to_string(),
            },
            plain_text: "Body".to_string(),
        }
    }

    fn app_config_with_mailboxes(ids: Vec<&str>) -> AppConfig {
        AppConfig {
            version: 1,
            database: DatabaseConfig {
                host: "postgres".to_string(),
                port: 5432,
                username: "user".to_string(),
                password: "secret".to_string(),
                database: "ai_memmail".to_string(),
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "json".to_string(),
                verbose_actions: true,
                retention_days: 180,
            },
            prompts: PromptConfig {
                root: "prompts".into(),
                safety_scan: "safety.md".into(),
                email_classifier: "email-classifier.md".into(),
                rule_action: "rule-action.md".into(),
            },
            ai: AiConfig {
                protocol: AiProtocol::Openai,
                api_url: "https://api.example/v1".to_string(),
                api_secret: "secret".to_string(),
                model: "model".to_string(),
                review: ReviewConfig {
                    enabled: false,
                    prompt_path: "review.md".into(),
                },
            },
            mcp_servers: Default::default(),
            mailboxes: ids
                .into_iter()
                .map(|id| MailboxConfig {
                    id: id.to_string(),
                    address: format!("{id}@example.com"),
                    enabled: true,
                    poll_interval_seconds: 30,
                    safety_forward_to: vec!["human@example.com".to_string()],
                    mcp_servers: vec![],
                    agent: AgentConfig {
                        system_prompt_path: "agent.md".into(),
                        default_forward_to: vec!["human@example.com".to_string()],
                    },
                    imap: ImapConfig {
                        host: "imap.example.com".to_string(),
                        port: 993,
                        tls: true,
                        username: format!("{id}@example.com"),
                        password: "secret".to_string(),
                        folder: "INBOX".to_string(),
                    },
                    smtp: SmtpConfig {
                        host: "smtp.example.com".to_string(),
                        port: 587,
                        starttls: true,
                        username: format!("{id}@example.com"),
                        password: "secret".to_string(),
                        from: format!("{id}@example.com"),
                    },
                })
                .collect(),
            banned_senders: vec![],
        }
    }
}
