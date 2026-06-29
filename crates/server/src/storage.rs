use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::config::DatabaseConfig;
use crate::logging::{ActionEvent, ActionLogger, LogLevel};
use crate::mail::{DedupeKey, InboundMessage, OutboundActionKind};

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
}

#[derive(Debug, Default, Clone)]
pub struct MemoryProcessingStore {
    statuses: Arc<Mutex<HashMap<DedupeKey, String>>>,
}

impl MemoryProcessingStore {
    pub fn status(&self, key: &DedupeKey) -> Option<String> {
        self.statuses
            .lock()
            .expect("memory processing store poisoned")
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
        let message_uid = event.message_uid.map(|uid| uid as i64);
        self.client
            .execute(
                "INSERT INTO action_logs
                (level, run_id, mailbox_id, message_uid, action, status, duration_ms, detail)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &log_level_value(event.level),
                    &event.run_id,
                    &event.mailbox_id,
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
}

#[async_trait::async_trait]
impl ProcessingStore for PgStore {
    async fn claim_message(
        &self,
        run_id: &str,
        message: &InboundMessage,
    ) -> Result<ProcessingClaim, StorageError> {
        let run_id = uuid::Uuid::parse_str(run_id)?.to_string();
        let key = message.metadata.dedupe_key();
        let inserted = self
            .client
            .query_opt(
                "INSERT INTO processing_runs
                (run_id, mailbox_id, uid_validity, uid, message_id, from_addr, subject, status)
                VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8)
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
                    SET run_id = $1::uuid, status = $2, message_id = $3, from_addr = $4,
                        subject = $5, updated_at = now()
                    WHERE mailbox_id = $6 AND uid_validity = $7 AND uid = $8
                        AND (status = $9 OR (status = $2 AND updated_at < now() - make_interval(mins => $10::int)))
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
    status == PROCESSING_STATUS_SEND_FAILED || (status == PROCESSING_STATUS_PROCESSING && stale)
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
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS banned_senders"));
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
    async fn memory_processing_store_reclaims_send_failures() {
        let store = MemoryProcessingStore::default();
        let message = message(43);
        let key = message.metadata.dedupe_key();

        store.claim_message("run-test", &message).await.unwrap();
        store
            .update_message_status(&key, PROCESSING_STATUS_SEND_FAILED, None)
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
