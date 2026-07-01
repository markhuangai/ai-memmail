use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::ai::AgentDecision;
use crate::config::DatabaseConfig;
use crate::logging::{ActionEvent, ActionLogger, LogLevel};
use crate::mail::{DedupeKey, InboundMessage, OutboundAction, OutboundActionKind};
use crate::safety::SafetyCategory;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("postgres connection failed: {0}")]
    Connect(#[from] tokio_postgres::Error),
    #[error("processing run id is not a uuid: {0}")]
    InvalidRunId(#[from] uuid::Error),
    #[error("processing store lock poisoned")]
    LockPoisoned,
}

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");
pub const PROCESSING_STATUS_PROCESSING: &str = "processing";
pub const PROCESSING_STATUS_RETRYABLE_FAILED: &str = "retryable_failed";
pub const PROCESSING_STATUS_SEND_FAILED: &str = "send_failed";
pub const PROCESSING_STALE_AFTER_MINUTES: i32 = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingClaim {
    Claimed,
    InProgress { status: String },
    AlreadyFinished { status: String },
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
    pub message_id: Option<String>,
    pub from_addr: String,
    pub subject: String,
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

#[derive(Debug)]
pub struct PgStore {
    client: tokio_postgres::Client,
}

impl PgStore {
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, StorageError> {
        let mut postgres_config = tokio_postgres::Config::new();
        postgres_config
            .host(&config.host)
            .port(config.port)
            .user(&config.username)
            .password(&config.password)
            .dbname(&config.database);
        let (client, connection) = postgres_config.connect(tokio_postgres::NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::error!(%error, "postgres connection task failed");
            }
        });
        Ok(Self { client })
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        self.client.batch_execute(INIT_SQL).await?;
        Ok(())
    }

    pub async fn insert_action_log(&self, event: &ActionEvent) -> Result<(), StorageError> {
        let message_uid_validity = event
            .message_uid_validity
            .map(|uid_validity| uid_validity as i64);
        let message_uid = event.message_uid.map(|uid| uid as i64);
        self.client
            .execute(
                "INSERT INTO action_logs
                (level, run_id, mailbox_id, message_uid_validity, message_uid, action, status, duration_ms, detail)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &log_level_value(event.level),
                    &event.run_id,
                    &event.mailbox_id,
                    &message_uid_validity,
                    &message_uid,
                    &event.action,
                    &event.status,
                    &(event.duration_ms as i64),
                    &event.detail,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn list_processed_emails(
        &self,
        limit: i64,
    ) -> Result<Vec<ProcessedEmail>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT run_id::text, mailbox_id, uid_validity, uid, message_id, from_addr,
                    subject, status, safety_category, safety_reason, agent_action,
                    agent_safety_notes, outbound_action, outbound_recipients, outbound_subject,
                    outbound_body, outbound_body_redacted, outbound_reason,
                    outbound_review_status, outbound_review_reason, created_at::text,
                    updated_at::text
                FROM processing_runs
                ORDER BY updated_at DESC
                LIMIT $1",
                &[&limit],
            )
            .await?;
        let mut emails = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: String = row.get(0);
            let mailbox_id: String = row.get(1);
            let uid_validity: i64 = row.get(2);
            let uid: i64 = row.get(3);
            let logs = self
                .list_processed_email_logs(&run_id, &mailbox_id, uid_validity, uid)
                .await?;
            emails.push(ProcessedEmail {
                run_id,
                mailbox_id,
                uid_validity: uid_validity as u64,
                uid: uid as u64,
                message_id: row.get(4),
                from_addr: row.get(5),
                subject: row.get(6),
                status: row.get(7),
                safety_category: row.get(8),
                safety_reason: row.get(9),
                agent_action: row.get(10),
                agent_safety_notes: row.get(11),
                outbound_action: row.get(12),
                outbound_recipients: row.get(13),
                outbound_subject: row.get(14),
                outbound_body: row.get(15),
                outbound_body_redacted: row.get(16),
                outbound_reason: row.get(17),
                outbound_review_status: row.get(18),
                outbound_review_reason: row.get(19),
                created_at: row.get(20),
                updated_at: row.get(21),
                logs,
            });
        }
        Ok(emails)
    }

    async fn list_processed_email_logs(
        &self,
        run_id: &str,
        mailbox_id: &str,
        uid_validity: i64,
        uid: i64,
    ) -> Result<Vec<ProcessedEmailLog>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT level, run_id, action, status, duration_ms, detail, created_at::text
                FROM action_logs
                WHERE run_id = $1
                    OR (
                        mailbox_id = $2
                        AND message_uid = $4
                        AND (message_uid_validity = $3 OR message_uid_validity IS NULL)
                    )
                ORDER BY created_at ASC, id ASC",
                &[&run_id, &mailbox_id, &uid_validity, &uid],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| ProcessedEmailLog {
                level: row.get(0),
                run_id: row.get(1),
                action: row.get(2),
                status: row.get(3),
                duration_ms: row.get::<_, i64>(4) as u64,
                detail: row.get(5),
                created_at: row.get(6),
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl ProcessingStore for PgStore {
    async fn claim_message(
        &self,
        run_id: &str,
        message: &InboundMessage,
    ) -> Result<ProcessingClaim, StorageError> {
        let run_id = parse_run_id(run_id)?;
        let key = message.metadata.dedupe_key();
        let inserted = self
            .client
            .query_opt(
                "INSERT INTO processing_runs
                (run_id, mailbox_id, uid_validity, uid, message_id, from_addr, subject, status)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (mailbox_id, uid_validity, uid) DO NOTHING
                RETURNING status",
                &[
                    &run_id,
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                    &message.metadata.message_id,
                    &message.metadata.from_addr,
                    &message.metadata.subject,
                    &PROCESSING_STATUS_PROCESSING,
                ],
            )
            .await?;
        if inserted.is_some() {
            return Ok(ProcessingClaim::Claimed);
        }

        let row = self
            .client
            .query_one(
                "SELECT status,
                updated_at < now() - make_interval(mins => $4::int) AS stale
                FROM processing_runs
                WHERE mailbox_id = $1 AND uid_validity = $2 AND uid = $3",
                &[
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                    &PROCESSING_STALE_AFTER_MINUTES,
                ],
            )
            .await?;
        let status: String = row.get(0);
        let stale: bool = row.get(1);
        if processing_status_can_reclaim(&status, stale) {
            let updated = self
                .client
                .query_opt(
                    "UPDATE processing_runs
                    SET run_id = $1, status = $2, message_id = $3, from_addr = $4,
                        subject = $5, updated_at = now()
                    WHERE mailbox_id = $6 AND uid_validity = $7 AND uid = $8
                        AND (status IN ($9, $10) OR (status = $2 AND updated_at < now() - make_interval(mins => $11::int)))
                    RETURNING status",
                    &[
                        &run_id,
                        &PROCESSING_STATUS_PROCESSING,
                        &message.metadata.message_id,
                        &message.metadata.from_addr,
                        &message.metadata.subject,
                        &key.mailbox_id,
                        &(key.uid_validity as i64),
                        &(key.uid as i64),
                        &PROCESSING_STATUS_SEND_FAILED,
                        &PROCESSING_STATUS_RETRYABLE_FAILED,
                        &PROCESSING_STALE_AFTER_MINUTES,
                    ],
                )
                .await?;
            if updated.is_some() {
                return Ok(ProcessingClaim::Claimed);
            }
        }

        Ok(processing_claim_for_existing_status(&status))
    }

    async fn update_message_status(
        &self,
        key: &DedupeKey,
        status: &str,
        outbound_action: Option<&OutboundActionKind>,
    ) -> Result<(), StorageError> {
        let outbound_action = outbound_action.map(outbound_action_value);
        self.client
            .execute(
                "UPDATE processing_runs
                SET status = $1, outbound_action = $2, updated_at = now()
                WHERE mailbox_id = $3 AND uid_validity = $4 AND uid = $5",
                &[
                    &status,
                    &outbound_action,
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                ],
            )
            .await?;
        Ok(())
    }

    async fn record_safety_result(
        &self,
        key: &DedupeKey,
        category: &SafetyCategory,
        reason: &str,
    ) -> Result<(), StorageError> {
        let category = safety_category_value(category);
        self.client
            .execute(
                "UPDATE processing_runs
                SET safety_category = $1, safety_reason = $2, updated_at = now()
                WHERE mailbox_id = $3 AND uid_validity = $4 AND uid = $5",
                &[
                    &category,
                    &reason,
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                ],
            )
            .await?;
        Ok(())
    }

    async fn record_agent_decision(
        &self,
        key: &DedupeKey,
        decision: &AgentDecision,
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "UPDATE processing_runs
                SET agent_action = $1, agent_safety_notes = $2, updated_at = now()
                WHERE mailbox_id = $3 AND uid_validity = $4 AND uid = $5",
                &[
                    &outbound_action_value(&decision.action.kind),
                    &decision.safety_notes,
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                ],
            )
            .await?;
        Ok(())
    }

    async fn record_outbound_action(
        &self,
        key: &DedupeKey,
        action: &OutboundAction,
    ) -> Result<(), StorageError> {
        let (body, body_redacted) = outbound_body_for_storage(action);
        self.client
            .execute(
                "UPDATE processing_runs
                SET outbound_action = $1, outbound_recipients = $2, outbound_subject = $3,
                    outbound_body = $4, outbound_body_redacted = $5, outbound_reason = $6,
                    updated_at = now()
                WHERE mailbox_id = $7 AND uid_validity = $8 AND uid = $9",
                &[
                    &outbound_action_value(&action.kind),
                    &action.recipients,
                    &empty_string_as_none(&action.subject),
                    &body,
                    &body_redacted,
                    &empty_string_as_none(&action.reason),
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                ],
            )
            .await?;
        Ok(())
    }

    async fn record_outbound_review(
        &self,
        key: &DedupeKey,
        status: &str,
        reason: &str,
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "UPDATE processing_runs
                SET outbound_review_status = $1, outbound_review_reason = $2, updated_at = now()
                WHERE mailbox_id = $3 AND uid_validity = $4 AND uid = $5",
                &[
                    &status,
                    &reason,
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                ],
            )
            .await?;
        Ok(())
    }

    async fn upsert_sender_review(
        &self,
        sender: &str,
        mailbox_id: &str,
        reason: &str,
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "INSERT INTO sender_reviews (sender, mailbox_id, reason, status)
                VALUES ($1, $2, $3, 'pending')
                ON CONFLICT (sender) DO UPDATE
                SET mailbox_id = EXCLUDED.mailbox_id,
                    reason = EXCLUDED.reason,
                    status = 'pending',
                    updated_at = now()",
                &[&sender, &mailbox_id, &reason],
            )
            .await?;
        Ok(())
    }
}

fn parse_run_id(run_id: &str) -> Result<uuid::Uuid, StorageError> {
    Ok(uuid::Uuid::parse_str(run_id)?)
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

#[async_trait::async_trait]
impl ActionLogger for PgStore {
    async fn log(&self, event: ActionEvent) {
        if let Err(error) = self.insert_action_log(&event).await {
            tracing::error!(%error, ?event, "failed to persist action log");
        }
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

fn outbound_body_for_storage(action: &OutboundAction) -> (Option<&str>, bool) {
    match action.kind {
        OutboundActionKind::Reply => (Some(action.body.as_str()), false),
        OutboundActionKind::Forward => (None, true),
        OutboundActionKind::Noop => (None, false),
    }
}

fn empty_string_as_none(value: &str) -> Option<&str> {
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

fn log_level_value(level: LogLevel) -> &'static str {
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
        };
        assert_eq!(outbound_body_for_storage(&reply), (Some("Answer"), false));

        let forward = OutboundAction {
            kind: OutboundActionKind::Forward,
            recipients: vec!["human@example.com".to_string()],
            subject: "Fwd: Question".to_string(),
            body: "contains original inbound body".to_string(),
            reason: "human review".to_string(),
        };
        assert_eq!(outbound_body_for_storage(&forward), (None, true));
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
                reason: "known answer".to_string()
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
                from_addr: "person@example.com".to_string(),
                subject: "Question".to_string(),
            },
            plain_text: "Body".to_string(),
        }
    }
}
