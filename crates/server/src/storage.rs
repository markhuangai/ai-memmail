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
use crate::mail::{
    DedupeKey, InboundMessage, OutboundAction, OutboundActionKind, SentFetchBatch, SentSyncCursor,
    ThreadContext,
};
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
    #[error("handoff source message not found: {0}")]
    HandoffSourceNotFound(String),
    #[error("invalid handoff: {0}")]
    InvalidHandoff(String),
}

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");
pub const HISTORY_BODY_THREADING_SQL: &str =
    include_str!("../migrations/002_history_body_threading.sql");
pub const EMAIL_CLASSIFICATION_RULES_SQL: &str =
    include_str!("../migrations/003_email_classification_rules.sql");
pub const DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL: &str =
    include_str!("../migrations/004_default_email_rule_seed_uniqueness.sql");
pub const SENT_THREAD_CONTEXT_SQL: &str = include_str!("../migrations/005_sent_thread_context.sql");
pub const THREAD_HANDOFFS_SQL: &str = include_str!("../migrations/006_thread_handoffs.sql");
pub const OUTBOUND_HTML_BODY_SQL: &str = include_str!("../migrations/007_outbound_html_body.sql");
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
    Migration {
        version: 5,
        name: "005_sent_thread_context",
        sql: SENT_THREAD_CONTEXT_SQL,
    },
    Migration {
        version: 6,
        name: "006_thread_handoffs",
        sql: THREAD_HANDOFFS_SQL,
    },
    Migration {
        version: 7,
        name: "007_outbound_html_body",
        sql: OUTBOUND_HTML_BODY_SQL,
    },
];
pub const PROCESSING_STATUS_PROCESSING: &str = "processing";
pub const PROCESSING_STATUS_RETRYABLE_FAILED: &str = "retryable_failed";
pub const PROCESSING_STATUS_SEND_FAILED: &str = "send_failed";
pub const PROCESSING_STATUS_HANDED_OFF: &str = "handed_off";
pub const PROCESSING_STALE_AFTER_MINUTES: i32 = 3;
pub const INBOUND_BODY_STORAGE_MAX_CHARS: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingClaim {
    Claimed,
    InProgress { status: String },
    AlreadyFinished { status: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentSyncState {
    pub cursor: SentSyncCursor,
    pub initial_backfill_complete: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadHandoffSummary {
    pub state: String,
    pub destination: String,
    pub remote_target: String,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHandoff {
    pub mailbox_id: String,
    pub thread_id: String,
    pub destination: String,
    pub remote_target: String,
    pub state: String,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewThreadHandoffDelivery {
    pub request_id: uuid::Uuid,
    pub mailbox_id: String,
    pub thread_id: String,
    pub source_run_id: Option<uuid::Uuid>,
    pub destination: String,
    pub remote_target: String,
    pub outbound_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHandoffDelivery {
    pub request_id: uuid::Uuid,
    pub mailbox_id: String,
    pub thread_id: String,
    pub source_run_id: Option<uuid::Uuid>,
    pub destination: String,
    pub remote_target: String,
    pub outbound_message_id: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHandoffSource {
    pub run_id: uuid::Uuid,
    pub mailbox_id: String,
    pub thread_id: String,
    pub uid_validity: u64,
    pub uid: u64,
    pub subject: String,
    pub status: String,
    pub safety_category: Option<String>,
    pub inbound_body_truncated: bool,
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

    async fn touch_processing(&self, _key: &DedupeKey) -> Result<(), StorageError> {
        Ok(())
    }

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

    async fn sent_sync_state(
        &self,
        _mailbox_id: &str,
    ) -> Result<Option<SentSyncState>, StorageError> {
        Ok(None)
    }

    async fn record_sent_batch(
        &self,
        _mailbox_id: &str,
        _backfill_cutoff: i64,
        _batch: &SentFetchBatch,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn load_thread_context(
        &self,
        _mailbox: &crate::config::MailboxConfig,
        message: &InboundMessage,
    ) -> Result<ThreadContext, StorageError> {
        Ok(ThreadContext::empty(message.metadata.thread_id()))
    }

    async fn active_thread_handoff(
        &self,
        _mailbox_id: &str,
        _thread_id: &str,
    ) -> Result<Option<ThreadHandoff>, StorageError> {
        Ok(None)
    }

    async fn begin_thread_handoff_delivery(
        &self,
        delivery: &NewThreadHandoffDelivery,
    ) -> Result<ThreadHandoffDelivery, StorageError> {
        Ok(ThreadHandoffDelivery {
            request_id: delivery.request_id,
            mailbox_id: delivery.mailbox_id.clone(),
            thread_id: delivery.thread_id.clone(),
            source_run_id: delivery.source_run_id,
            destination: delivery.destination.clone(),
            remote_target: delivery.remote_target.clone(),
            outbound_message_id: delivery.outbound_message_id.clone(),
            status: "sending".to_string(),
            error: None,
        })
    }

    async fn finish_thread_handoff_delivery(
        &self,
        _mailbox_id: &str,
        _thread_id: &str,
        _request_id: uuid::Uuid,
        _status: &str,
        _error: Option<&str>,
    ) -> Result<(), StorageError> {
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
    handoffs: Arc<Mutex<HashMap<(String, String), ThreadHandoff>>>,
    handoff_deliveries: Arc<Mutex<HashMap<(String, String, uuid::Uuid), ThreadHandoffDelivery>>>,
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
    pub html_body: Option<String>,
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
    pub outbound_body_html: Option<String>,
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
    pub handoff: Option<ThreadHandoffSummary>,
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
include!("storage/memory_store.rs");

include!("storage/value_helpers.rs");

#[cfg(test)]
mod tests;
