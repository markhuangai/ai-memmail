use std::collections::HashMap;

use crate::ai::AgentDecision;
use crate::config::DatabaseConfig;
use crate::logging::{ActionEvent, ActionLogger};
use crate::mail::{DedupeKey, InboundMessage, OutboundAction, OutboundActionKind};
use crate::safety::SafetyCategory;
use crate::storage::{
    empty_string_as_none, log_level_value, migration_checksum, outbound_action_value,
    outbound_body_for_storage, parse_run_id, processing_claim_for_existing_status,
    processing_status_can_reclaim, safety_category_value, validate_applied_migration,
    AppliedMigration, Migration, ProcessedEmail, ProcessedEmailLog, ProcessingClaim,
    ProcessingStore, StorageError, MIGRATIONS, MIGRATION_LOCK_ID, PROCESSING_STALE_AFTER_MINUTES,
    PROCESSING_STATUS_PROCESSING, PROCESSING_STATUS_RETRYABLE_FAILED,
    PROCESSING_STATUS_SEND_FAILED, SCHEMA_MIGRATIONS_SQL,
};

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
        self.client
            .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_ID])
            .await?;
        let result = self.apply_migrations().await;
        let unlock_result = self
            .client
            .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_ID])
            .await;

        match (result, unlock_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error.into()),
            (Ok(()), Ok(_)) => Ok(()),
        }
    }

    async fn apply_migrations(&self) -> Result<(), StorageError> {
        self.client.batch_execute(SCHEMA_MIGRATIONS_SQL).await?;
        let applied = self.applied_migrations().await?;
        for migration in MIGRATIONS {
            let checksum = migration_checksum(migration.sql);
            match applied.get(&migration.version) {
                Some(applied) => {
                    validate_applied_migration(migration, &checksum, applied)?;
                }
                None => {
                    self.apply_pending_migration(migration, &checksum).await?;
                }
            }
        }
        Ok(())
    }

    async fn applied_migrations(&self) -> Result<HashMap<i32, AppliedMigration>, StorageError> {
        let rows = self
            .client
            .query("SELECT version, name, checksum FROM schema_migrations", &[])
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.get(0),
                    AppliedMigration {
                        name: row.get(1),
                        checksum: row.get(2),
                    },
                )
            })
            .collect())
    }

    async fn apply_pending_migration(
        &self,
        migration: &Migration,
        checksum: &str,
    ) -> Result<(), StorageError> {
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            self.client.batch_execute(migration.sql).await?;
            self.client
                .execute(
                    "INSERT INTO schema_migrations (version, name, checksum)
                    VALUES ($1, $2, $3)",
                    &[&migration.version, &migration.name, &checksum],
                )
                .await?;
            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                self.client.batch_execute("COMMIT").await?;
                Ok(())
            }
            Err(error) => {
                let _ = self.client.batch_execute("ROLLBACK").await;
                Err(error)
            }
        }
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

#[async_trait::async_trait]
impl ActionLogger for PgStore {
    async fn log(&self, event: ActionEvent) {
        if let Err(error) = self.insert_action_log(&event).await {
            tracing::error!(%error, ?event, "failed to persist action log");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::LogLevel;
    use crate::mail::MessageMetadata;
    use crate::storage::INIT_SQL;

    #[tokio::test]
    async fn pg_store_migrates_idempotently_and_tracks_checksum() {
        let Some(pg) = TestPgStore::create().await else {
            return;
        };

        pg.store.migrate().await.unwrap();
        pg.store.migrate().await.unwrap();

        let rows = pg
            .store
            .client
            .query(
                "SELECT version, name, checksum FROM schema_migrations ORDER BY version",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<_, i32>(0), 1);
        assert_eq!(rows[0].get::<_, String>(1), "001_init");
        assert_eq!(rows[0].get::<_, String>(2), migration_checksum(INIT_SQL));

        pg.cleanup().await;
    }

    #[tokio::test]
    async fn pg_store_claims_reclaims_retryable_and_skips_finished_messages() {
        let Some(pg) = TestPgStore::create().await else {
            return;
        };
        pg.store.migrate().await.unwrap();

        let message = message(70);
        let key = message.metadata.dedupe_key();
        assert_eq!(
            pg.store
                .claim_message(&uuid::Uuid::new_v4().to_string(), &message)
                .await
                .unwrap(),
            ProcessingClaim::Claimed
        );
        assert_eq!(
            pg.store
                .claim_message(&uuid::Uuid::new_v4().to_string(), &message)
                .await
                .unwrap(),
            ProcessingClaim::InProgress {
                status: PROCESSING_STATUS_PROCESSING.to_string()
            }
        );

        pg.store
            .update_message_status(&key, PROCESSING_STATUS_SEND_FAILED, None)
            .await
            .unwrap();
        assert_eq!(
            pg.store
                .claim_message(&uuid::Uuid::new_v4().to_string(), &message)
                .await
                .unwrap(),
            ProcessingClaim::Claimed
        );

        pg.store
            .update_message_status(&key, "replied", Some(&OutboundActionKind::Reply))
            .await
            .unwrap();
        assert_eq!(
            pg.store
                .claim_message(&uuid::Uuid::new_v4().to_string(), &message)
                .await
                .unwrap(),
            ProcessingClaim::AlreadyFinished {
                status: "replied".to_string()
            }
        );

        pg.cleanup().await;
    }

    #[tokio::test]
    async fn pg_store_records_processed_email_history_and_logs() {
        let Some(pg) = TestPgStore::create().await else {
            return;
        };
        pg.store.migrate().await.unwrap();

        let run_id = uuid::Uuid::new_v4().to_string();
        let message = message(71);
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
            safety_notes: "safe to answer".to_string(),
        };

        assert_eq!(
            pg.store.claim_message(&run_id, &message).await.unwrap(),
            ProcessingClaim::Claimed
        );
        pg.store
            .record_safety_result(&key, &SafetyCategory::Safe, "routine")
            .await
            .unwrap();
        pg.store
            .record_agent_decision(&key, &decision)
            .await
            .unwrap();
        pg.store
            .record_outbound_action(&key, &action)
            .await
            .unwrap();
        pg.store
            .record_outbound_review(&key, "approved", "looks safe")
            .await
            .unwrap();
        pg.store
            .update_message_status(&key, "replied", Some(&OutboundActionKind::Reply))
            .await
            .unwrap();

        pg.store
            .insert_action_log(&ActionEvent {
                level: LogLevel::Info,
                run_id: run_id.clone(),
                mailbox_id: Some(key.mailbox_id.clone()),
                message_uid_validity: Some(key.uid_validity),
                message_uid: Some(key.uid),
                action: "processing_claim".to_string(),
                status: "claimed".to_string(),
                duration_ms: 7,
                detail: Some("claimed message".to_string()),
            })
            .await
            .unwrap();
        pg.store
            .insert_action_log(&ActionEvent {
                level: LogLevel::Warn,
                run_id: "legacy-run".to_string(),
                mailbox_id: Some(key.mailbox_id.clone()),
                message_uid_validity: None,
                message_uid: Some(key.uid),
                action: "legacy_retry".to_string(),
                status: "matched".to_string(),
                duration_ms: 8,
                detail: None,
            })
            .await
            .unwrap();
        pg.store
            .insert_action_log(&ActionEvent {
                level: LogLevel::Error,
                run_id: "other-run".to_string(),
                mailbox_id: Some(key.mailbox_id.clone()),
                message_uid_validity: Some(key.uid_validity + 1),
                message_uid: Some(key.uid),
                action: "wrong_uidvalidity".to_string(),
                status: "ignored".to_string(),
                duration_ms: 9,
                detail: None,
            })
            .await
            .unwrap();

        let emails = pg.store.list_processed_emails(10).await.unwrap();
        assert_eq!(emails.len(), 1);
        let email = &emails[0];
        assert_eq!(email.run_id, run_id);
        assert_eq!(email.mailbox_id, "support");
        assert_eq!(email.uid_validity, 1);
        assert_eq!(email.uid, 71);
        assert_eq!(email.message_id, Some("<71@example.com>".to_string()));
        assert_eq!(email.from_addr, "person@example.com");
        assert_eq!(email.subject, "Question");
        assert_eq!(email.status, "replied");
        assert_eq!(email.safety_category, Some("safe".to_string()));
        assert_eq!(email.safety_reason, Some("routine".to_string()));
        assert_eq!(email.agent_action, Some("reply".to_string()));
        assert_eq!(email.agent_safety_notes, Some("safe to answer".to_string()));
        assert_eq!(email.outbound_action, Some("reply".to_string()));
        assert_eq!(email.outbound_recipients, vec!["person@example.com"]);
        assert_eq!(email.outbound_subject, Some("Re: Question".to_string()));
        assert_eq!(email.outbound_body, Some("Answer".to_string()));
        assert!(!email.outbound_body_redacted);
        assert_eq!(email.outbound_reason, Some("known answer".to_string()));
        assert_eq!(email.outbound_review_status, Some("approved".to_string()));
        assert_eq!(email.outbound_review_reason, Some("looks safe".to_string()));
        assert_eq!(
            email
                .logs
                .iter()
                .map(|log| log.action.as_str())
                .collect::<Vec<_>>(),
            vec!["processing_claim", "legacy_retry"]
        );
        assert_eq!(email.logs[0].duration_ms, 7);
        assert_eq!(email.logs[0].detail, Some("claimed message".to_string()));

        pg.cleanup().await;
    }

    #[tokio::test]
    async fn pg_store_redacts_forward_body_and_records_sender_review() {
        let Some(pg) = TestPgStore::create().await else {
            return;
        };
        pg.store.migrate().await.unwrap();

        let run_id = uuid::Uuid::new_v4().to_string();
        let message = message(72);
        let key = message.metadata.dedupe_key();
        let action = OutboundAction {
            kind: OutboundActionKind::Forward,
            recipients: vec!["human@example.com".to_string()],
            subject: "Fwd: Question".to_string(),
            body: "contains the inbound body".to_string(),
            reason: "needs human review".to_string(),
        };

        pg.store.claim_message(&run_id, &message).await.unwrap();
        pg.store
            .record_outbound_action(&key, &action)
            .await
            .unwrap();
        pg.store
            .upsert_sender_review(
                &message.metadata.from_addr,
                &message.metadata.mailbox_id,
                "needs human review",
            )
            .await
            .unwrap();

        let email = pg
            .store
            .list_processed_emails(1)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(email.outbound_action, Some("forward".to_string()));
        assert_eq!(email.outbound_body, None);
        assert!(email.outbound_body_redacted);

        let row = pg
            .store
            .client
            .query_one(
                "SELECT mailbox_id, reason, status FROM sender_reviews WHERE sender = $1",
                &[&message.metadata.from_addr],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), "support");
        assert_eq!(row.get::<_, String>(1), "needs human review");
        assert_eq!(row.get::<_, String>(2), "pending");

        pg.cleanup().await;
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

    struct TestPgStore {
        store: PgStore,
        admin_config: DatabaseConfig,
        database_name: String,
    }

    impl TestPgStore {
        async fn create() -> Option<Self> {
            let admin_config = test_pg_admin_config()?;
            let database_name = format!("ai_memmail_test_{}", uuid::Uuid::new_v4().simple());
            let admin = connect_test_pg(&admin_config).await;
            admin
                .batch_execute(&format!("CREATE DATABASE {}", quote_ident(&database_name)))
                .await
                .unwrap();

            let mut store_config = admin_config.clone();
            store_config.database = database_name.clone();
            let store = PgStore::connect(&store_config).await.unwrap();
            Some(Self {
                store,
                admin_config,
                database_name,
            })
        }

        async fn cleanup(self) {
            let database_name = self.database_name;
            let admin_config = self.admin_config;
            drop(self.store);

            let admin = connect_test_pg(&admin_config).await;
            admin
                .batch_execute(&format!(
                    "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                    quote_ident(&database_name)
                ))
                .await
                .unwrap();
        }
    }

    fn test_pg_admin_config() -> Option<DatabaseConfig> {
        if std::env::var("AI_MEMMAIL_RUN_POSTGRES_TESTS")
            .ok()
            .as_deref()
            != Some("1")
        {
            return None;
        }
        let host = std::env::var("AI_MEMMAIL_TEST_PG_HOST").ok()?;
        let port = std::env::var("AI_MEMMAIL_TEST_PG_PORT")
            .ok()
            .and_then(|value| value.parse().ok())?;
        let username = std::env::var("AI_MEMMAIL_TEST_PG_USER").ok()?;
        let password = std::env::var("AI_MEMMAIL_TEST_PG_PASSWORD").ok()?;
        let database = std::env::var("AI_MEMMAIL_TEST_PG_DATABASE").ok()?;
        Some(DatabaseConfig {
            host,
            port,
            username,
            password,
            database,
        })
    }

    async fn connect_test_pg(config: &DatabaseConfig) -> tokio_postgres::Client {
        let mut postgres_config = tokio_postgres::Config::new();
        postgres_config
            .host(&config.host)
            .port(config.port)
            .user(&config.username)
            .password(&config.password)
            .dbname(&config.database);
        let (client, connection) = postgres_config
            .connect(tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
    }

    fn quote_ident(identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}
