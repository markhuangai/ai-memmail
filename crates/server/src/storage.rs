use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ai::AgentDecision;
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
}

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");
pub const HISTORY_BODY_THREADING_SQL: &str =
    include_str!("../migrations/002_history_body_threading.sql");
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
}

#[derive(Debug, Default, Clone)]
pub struct MemoryProcessingStore {
    statuses: Arc<Mutex<HashMap<DedupeKey, String>>>,
    safety_results: Arc<Mutex<HashMap<DedupeKey, StoredSafetyResult>>>,
    sender_reviews: Arc<Mutex<HashMap<String, SenderReviewRecord>>>,
    agent_decisions: Arc<Mutex<HashMap<DedupeKey, StoredAgentDecision>>>,
    outbound_actions: Arc<Mutex<HashMap<DedupeKey, StoredOutboundAction>>>,
    outbound_reviews: Arc<Mutex<HashMap<DedupeKey, StoredOutboundReview>>>,
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
}
