use std::collections::HashMap;

use crate::ai::AgentDecision;
use crate::classification::{
    default_categories, default_topics, normalize_label_name, EmailCategory, EmailClassification,
    EmailClassificationConfig, EmailRule, EmailRuleAction, EmailTaxonomy, EmailTopic, NewEmailRule,
    ResolvedEmailClassification, DEFAULT_MARKETING_REPLY_GOAL,
};
use crate::config::{AppConfig, DatabaseConfig};
use crate::logging::{ActionEvent, ActionLogger};
use crate::mail::{DedupeKey, InboundMessage, OutboundAction, OutboundActionKind};
use crate::safety::SafetyCategory;
use crate::storage::{
    empty_string_as_none, inbound_body_for_storage, log_level_value, migration_checksum,
    outbound_action_value, outbound_body_for_storage, parse_run_id,
    processing_claim_for_existing_status, processing_status_can_reclaim, safety_category_value,
    validate_applied_migration, AppliedMigration, Migration, ProcessedEmail, ProcessedEmailLog,
    ProcessingClaim, ProcessingStore, StorageError, MIGRATIONS, MIGRATION_LOCK_ID,
    PROCESSING_STALE_AFTER_MINUTES, PROCESSING_STATUS_PROCESSING,
    PROCESSING_STATUS_RETRYABLE_FAILED, PROCESSING_STATUS_SEND_FAILED, SCHEMA_MIGRATIONS_SQL,
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
                "SELECT pr.run_id::text, pr.mailbox_id, pr.uid_validity, pr.uid,
                    COALESCE(pr.thread_id, pr.message_id, pr.mailbox_id || ':' || pr.uid_validity::text || ':' || pr.uid::text),
                    pr.message_id, pr.in_reply_to, pr.message_references, pr.from_addr, pr.subject,
                    pr.inbound_body, pr.inbound_body_truncated, pr.status, pr.safety_category, pr.safety_reason,
                    pr.agent_action, pr.agent_safety_notes, pr.outbound_action, pr.outbound_recipients, pr.outbound_subject,
                    pr.outbound_body, pr.outbound_body_redacted, pr.outbound_message_id, pr.outbound_reason,
                    pr.outbound_review_status, pr.outbound_review_reason,
                    c.name, COALESCE(
                        ARRAY(
                            SELECT t.name
                            FROM email_topics t
                            WHERE t.id = ANY(pr.classification_topic_ids)
                            ORDER BY t.name
                        ),
                        '{}'
                    ),
                    pr.classification_reason, pr.classification_confidence, pr.decision_source,
                    pr.matched_rule_id, pr.matched_rule_name, pr.matched_rule_goal,
                    pr.created_at::text, pr.updated_at::text
                FROM processing_runs pr
                LEFT JOIN email_categories c ON c.id = pr.classification_category_id
                ORDER BY pr.updated_at DESC
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
                thread_id: row.get(4),
                message_id: row.get(5),
                in_reply_to: row.get(6),
                references: row.get(7),
                from_addr: row.get(8),
                subject: row.get(9),
                inbound_body: row.get(10),
                inbound_body_truncated: row.get(11),
                status: row.get(12),
                safety_category: row.get(13),
                safety_reason: row.get(14),
                agent_action: row.get(15),
                agent_safety_notes: row.get(16),
                outbound_action: row.get(17),
                outbound_recipients: row.get(18),
                outbound_subject: row.get(19),
                outbound_body: row.get(20),
                outbound_body_redacted: row.get(21),
                outbound_message_id: row.get(22),
                outbound_reason: row.get(23),
                outbound_review_status: row.get(24),
                outbound_review_reason: row.get(25),
                classification_category: row.get(26),
                classification_topics: row.get(27),
                classification_reason: row.get(28),
                classification_confidence: row.get::<_, Option<i16>>(29).map(|value| value as u16),
                decision_source: row.get(30),
                matched_rule_id: row.get(31),
                matched_rule_name: row.get(32),
                matched_rule_goal: row.get(33),
                created_at: row.get(34),
                updated_at: row.get(35),
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

    async fn thread_id_for_message(
        &self,
        message: &InboundMessage,
    ) -> Result<String, StorageError> {
        let mut related_ids = message.metadata.references.clone();
        if let Some(in_reply_to) = &message.metadata.in_reply_to {
            related_ids.push(in_reply_to.clone());
        }
        related_ids.sort();
        related_ids.dedup();
        if related_ids.is_empty() {
            return Ok(message.metadata.thread_id());
        }

        let row = self
            .client
            .query_opt(
                "SELECT thread_id
                FROM processing_runs
                WHERE message_id = ANY($1) OR outbound_message_id = ANY($1)
                ORDER BY updated_at ASC
                LIMIT 1",
                &[&related_ids],
            )
            .await?;
        Ok(row
            .and_then(|row| row.get::<_, Option<String>>(0))
            .filter(|thread_id| !thread_id.trim().is_empty())
            .unwrap_or_else(|| message.metadata.thread_id()))
    }

    pub async fn list_email_classification_config(
        &self,
    ) -> Result<EmailClassificationConfig, StorageError> {
        Ok(EmailClassificationConfig {
            categories: self.list_email_categories().await?,
            topics: self.list_email_topics().await?,
            rules: self.list_email_rules().await?,
        })
    }

    async fn list_email_categories(&self) -> Result<Vec<EmailCategory>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT id, name, description, status, source, created_at::text, updated_at::text
                FROM email_categories
                ORDER BY name ASC",
                &[],
            )
            .await?;
        Ok(rows.into_iter().map(email_category_from_row).collect())
    }

    async fn list_email_topics(&self) -> Result<Vec<EmailTopic>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT id, name, description, status, source, created_at::text, updated_at::text
                FROM email_topics
                ORDER BY name ASC",
                &[],
            )
            .await?;
        Ok(rows.into_iter().map(email_topic_from_row).collect())
    }

    async fn list_email_rules(&self) -> Result<Vec<EmailRule>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT r.id, r.mailbox_id, r.name, r.category_id, c.name,
                    COALESCE(array_remove(array_agg(rt.topic_id ORDER BY t.name), NULL), '{}'),
                    COALESCE(array_remove(array_agg(t.name ORDER BY t.name), NULL), '{}'),
                    r.action, r.reply_goal, r.enabled, r.priority, r.created_at::text, r.updated_at::text
                FROM email_rules r
                JOIN email_categories c ON c.id = r.category_id
                LEFT JOIN email_rule_topics rt ON rt.rule_id = r.id
                LEFT JOIN email_topics t ON t.id = rt.topic_id
                GROUP BY r.id, c.name
                ORDER BY r.mailbox_id ASC, r.priority ASC, r.id ASC",
                &[],
            )
            .await?;
        rows.into_iter().map(email_rule_from_row).collect()
    }

    pub async fn create_email_category(
        &self,
        name: &str,
        description: &str,
    ) -> Result<EmailCategory, StorageError> {
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_one(
                "INSERT INTO email_categories (name, description, source)
                VALUES ($1, $2, 'user')
                ON CONFLICT (name) DO UPDATE
                SET description = EXCLUDED.description,
                    status = 'active',
                    updated_at = now()
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description],
            )
            .await?;
        Ok(email_category_from_row(row))
    }

    pub async fn update_email_category(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<EmailCategory, StorageError> {
        validate_label_status(status)?;
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_opt(
                "UPDATE email_categories
                SET name = $1, description = $2, status = $3, updated_at = now()
                WHERE id = $4
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description, &status, &id],
            )
            .await?
            .ok_or_else(|| StorageError::ClassificationNotFound(format!("category {id}")))?;
        Ok(email_category_from_row(row))
    }

    pub async fn delete_email_category(&self, id: i64) -> Result<(), StorageError> {
        let deleted = self
            .client
            .execute("DELETE FROM email_categories WHERE id = $1", &[&id])
            .await?;
        if deleted == 0 {
            return Err(StorageError::ClassificationNotFound(format!(
                "category {id}"
            )));
        }
        Ok(())
    }

    pub async fn merge_email_category(
        &self,
        source_id: i64,
        target_id: i64,
    ) -> Result<(), StorageError> {
        if source_id == target_id {
            return Err(StorageError::InvalidClassification(
                "source and target category must differ".to_string(),
            ));
        }
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            self.require_category(target_id).await?;
            self.require_category(source_id).await?;
            self.client
                .execute(
                    "UPDATE processing_runs
                    SET classification_category_id = $1, updated_at = now()
                    WHERE classification_category_id = $2",
                    &[&target_id, &source_id],
                )
                .await?;
            self.client
                .execute(
                    "UPDATE email_rules
                    SET category_id = $1, updated_at = now()
                    WHERE category_id = $2",
                    &[&target_id, &source_id],
                )
                .await?;
            self.client
                .execute("DELETE FROM email_categories WHERE id = $1", &[&source_id])
                .await?;
            Ok(())
        }
        .await;
        self.finish_transaction(result).await
    }

    pub async fn create_email_topic(
        &self,
        name: &str,
        description: &str,
    ) -> Result<EmailTopic, StorageError> {
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_one(
                "INSERT INTO email_topics (name, description, source)
                VALUES ($1, $2, 'user')
                ON CONFLICT (name) DO UPDATE
                SET description = EXCLUDED.description,
                    status = 'active',
                    updated_at = now()
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description],
            )
            .await?;
        Ok(email_topic_from_row(row))
    }

    pub async fn update_email_topic(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<EmailTopic, StorageError> {
        validate_label_status(status)?;
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_opt(
                "UPDATE email_topics
                SET name = $1, description = $2, status = $3, updated_at = now()
                WHERE id = $4
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description, &status, &id],
            )
            .await?
            .ok_or_else(|| StorageError::ClassificationNotFound(format!("topic {id}")))?;
        Ok(email_topic_from_row(row))
    }

    pub async fn delete_email_topic(&self, id: i64) -> Result<(), StorageError> {
        let deleted = self
            .client
            .execute("DELETE FROM email_topics WHERE id = $1", &[&id])
            .await?;
        if deleted == 0 {
            return Err(StorageError::ClassificationNotFound(format!("topic {id}")));
        }
        Ok(())
    }

    pub async fn merge_email_topic(
        &self,
        source_id: i64,
        target_id: i64,
    ) -> Result<(), StorageError> {
        if source_id == target_id {
            return Err(StorageError::InvalidClassification(
                "source and target topic must differ".to_string(),
            ));
        }
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            self.require_topic(target_id).await?;
            self.require_topic(source_id).await?;
            self.client
                .execute(
                    "UPDATE processing_runs
                    SET classification_topic_ids = ARRAY(
                        SELECT DISTINCT CASE WHEN topic_id = $1 THEN $2 ELSE topic_id END
                        FROM unnest(classification_topic_ids) AS topic_id
                    ),
                    updated_at = now()
                    WHERE $1 = ANY(classification_topic_ids)",
                    &[&source_id, &target_id],
                )
                .await?;
            self.client
                .execute(
                    "INSERT INTO email_rule_topics (rule_id, topic_id)
                    SELECT rule_id, $1 FROM email_rule_topics WHERE topic_id = $2
                    ON CONFLICT DO NOTHING",
                    &[&target_id, &source_id],
                )
                .await?;
            self.client
                .execute(
                    "DELETE FROM email_rule_topics WHERE topic_id = $1",
                    &[&source_id],
                )
                .await?;
            self.client
                .execute("DELETE FROM email_topics WHERE id = $1", &[&source_id])
                .await?;
            Ok(())
        }
        .await;
        self.finish_transaction(result).await
    }

    pub async fn create_email_rule(&self, rule: NewEmailRule) -> Result<EmailRule, StorageError> {
        validate_rule(&rule)?;
        self.client.batch_execute("BEGIN").await?;
        let result = self.insert_email_rule(rule).await;
        self.finish_transaction(result).await
    }

    pub async fn update_email_rule(
        &self,
        id: i64,
        rule: NewEmailRule,
    ) -> Result<EmailRule, StorageError> {
        validate_rule(&rule)?;
        self.client.batch_execute("BEGIN").await?;
        let result: Result<EmailRule, StorageError> = async {
            let row = self
                .client
                .query_opt(
                    "UPDATE email_rules
                    SET mailbox_id = $1, name = $2, category_id = $3, action = $4,
                        reply_goal = $5, enabled = $6, priority = $7, updated_at = now()
                    WHERE id = $8
                    RETURNING id",
                    &[
                        &rule.mailbox_id,
                        &rule.name,
                        &rule.category_id,
                        &rule.action.as_str(),
                        &rule.reply_goal,
                        &rule.enabled,
                        &rule.priority,
                        &id,
                    ],
                )
                .await?
                .ok_or_else(|| StorageError::ClassificationNotFound(format!("rule {id}")))?;
            let rule_id: i64 = row.get(0);
            self.replace_rule_topics(rule_id, &rule.topic_ids).await?;
            self.get_email_rule(rule_id).await
        }
        .await;
        self.finish_transaction(result).await
    }

    pub async fn delete_email_rule(&self, id: i64) -> Result<(), StorageError> {
        let deleted = self
            .client
            .execute("DELETE FROM email_rules WHERE id = $1", &[&id])
            .await?;
        if deleted == 0 {
            return Err(StorageError::ClassificationNotFound(format!("rule {id}")));
        }
        Ok(())
    }

    async fn insert_email_rule(&self, rule: NewEmailRule) -> Result<EmailRule, StorageError> {
        let row = self
            .client
            .query_one(
                "INSERT INTO email_rules
                (mailbox_id, name, category_id, action, reply_goal, enabled, priority)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id",
                &[
                    &rule.mailbox_id,
                    &rule.name,
                    &rule.category_id,
                    &rule.action.as_str(),
                    &rule.reply_goal,
                    &rule.enabled,
                    &rule.priority,
                ],
            )
            .await?;
        let rule_id: i64 = row.get(0);
        self.replace_rule_topics(rule_id, &rule.topic_ids).await?;
        self.get_email_rule(rule_id).await
    }

    async fn replace_rule_topics(
        &self,
        rule_id: i64,
        topic_ids: &[i64],
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "DELETE FROM email_rule_topics WHERE rule_id = $1",
                &[&rule_id],
            )
            .await?;
        for topic_id in topic_ids {
            self.client
                .execute(
                    "INSERT INTO email_rule_topics (rule_id, topic_id)
                    VALUES ($1, $2)
                    ON CONFLICT DO NOTHING",
                    &[&rule_id, topic_id],
                )
                .await?;
        }
        Ok(())
    }

    async fn get_email_rule(&self, id: i64) -> Result<EmailRule, StorageError> {
        let row = self
            .client
            .query_opt(
                "SELECT r.id, r.mailbox_id, r.name, r.category_id, c.name,
                    COALESCE(array_remove(array_agg(rt.topic_id ORDER BY t.name), NULL), '{}'),
                    COALESCE(array_remove(array_agg(t.name ORDER BY t.name), NULL), '{}'),
                    r.action, r.reply_goal, r.enabled, r.priority, r.created_at::text, r.updated_at::text
                FROM email_rules r
                JOIN email_categories c ON c.id = r.category_id
                LEFT JOIN email_rule_topics rt ON rt.rule_id = r.id
                LEFT JOIN email_topics t ON t.id = rt.topic_id
                WHERE r.id = $1
                GROUP BY r.id, c.name",
                &[&id],
            )
            .await?
            .ok_or_else(|| StorageError::ClassificationNotFound(format!("rule {id}")))?;
        email_rule_from_row(row)
    }

    async fn require_category(&self, id: i64) -> Result<(), StorageError> {
        let exists = self
            .client
            .query_opt("SELECT 1 FROM email_categories WHERE id = $1", &[&id])
            .await?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StorageError::ClassificationNotFound(format!(
                "category {id}"
            )))
        }
    }

    async fn require_topic(&self, id: i64) -> Result<(), StorageError> {
        let exists = self
            .client
            .query_opt("SELECT 1 FROM email_topics WHERE id = $1", &[&id])
            .await?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StorageError::ClassificationNotFound(format!("topic {id}")))
        }
    }

    async fn finish_transaction<T>(
        &self,
        result: Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        match result {
            Ok(value) => {
                self.client.batch_execute("COMMIT").await?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.client.batch_execute("ROLLBACK").await;
                Err(error)
            }
        }
    }
}

fn email_category_from_row(row: tokio_postgres::Row) -> EmailCategory {
    EmailCategory {
        id: row.get(0),
        name: row.get(1),
        description: row.get(2),
        status: row.get(3),
        source: row.get(4),
        created_at: row.get(5),
        updated_at: row.get(6),
    }
}

fn email_topic_from_row(row: tokio_postgres::Row) -> EmailTopic {
    EmailTopic {
        id: row.get(0),
        name: row.get(1),
        description: row.get(2),
        status: row.get(3),
        source: row.get(4),
        created_at: row.get(5),
        updated_at: row.get(6),
    }
}

fn email_rule_from_row(row: tokio_postgres::Row) -> Result<EmailRule, StorageError> {
    let action: String = row.get(7);
    Ok(EmailRule {
        id: row.get(0),
        mailbox_id: row.get(1),
        name: row.get(2),
        category_id: row.get(3),
        category: row.get(4),
        topic_ids: row.get(5),
        topics: row.get(6),
        action: EmailRuleAction::try_from(action.as_str())
            .map_err(StorageError::InvalidClassification)?,
        reply_goal: row.get(8),
        enabled: row.get(9),
        priority: row.get(10),
        created_at: row.get(11),
        updated_at: row.get(12),
    })
}

fn validate_label_status(status: &str) -> Result<(), StorageError> {
    if matches!(status, "active" | "archived") {
        Ok(())
    } else {
        Err(StorageError::InvalidClassification(format!(
            "label status must be active or archived, got {status}"
        )))
    }
}

fn validate_rule(rule: &NewEmailRule) -> Result<(), StorageError> {
    if rule.mailbox_id.trim().is_empty() {
        return Err(StorageError::InvalidClassification(
            "rule mailbox_id is required".to_string(),
        ));
    }
    if rule.name.trim().is_empty() {
        return Err(StorageError::InvalidClassification(
            "rule name is required".to_string(),
        ));
    }
    if !matches!(rule.action, EmailRuleAction::Noop) && rule.reply_goal.trim().is_empty() {
        return Err(StorageError::InvalidClassification(
            "reply_goal is required for reply and forward rules".to_string(),
        ));
    }
    Ok(())
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
        let thread_id = self.thread_id_for_message(message).await?;
        let (inbound_body, inbound_body_truncated) = inbound_body_for_storage(message);
        let inserted = self
            .client
            .query_opt(
                "INSERT INTO processing_runs
                (run_id, mailbox_id, uid_validity, uid, thread_id, message_id, in_reply_to,
                    message_references, from_addr, subject, inbound_body, inbound_body_truncated, status)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                ON CONFLICT (mailbox_id, uid_validity, uid) DO NOTHING
                RETURNING status",
                &[
                    &run_id,
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                    &thread_id,
                    &message.metadata.message_id,
                    &message.metadata.in_reply_to,
                    &message.metadata.references,
                    &message.metadata.from_addr,
                    &message.metadata.subject,
                    &inbound_body,
                    &inbound_body_truncated,
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
                    SET run_id = $1, status = $2, thread_id = $3, message_id = $4,
                        in_reply_to = $5, message_references = $6, from_addr = $7,
                        subject = $8, inbound_body = $9, inbound_body_truncated = $10,
                        updated_at = now()
                    WHERE mailbox_id = $11 AND uid_validity = $12 AND uid = $13
                        AND (status IN ($14, $15) OR (status = $2 AND updated_at < now() - make_interval(mins => $16::int)))
                    RETURNING status",
                    &[
                        &run_id,
                        &PROCESSING_STATUS_PROCESSING,
                        &thread_id,
                        &message.metadata.message_id,
                        &message.metadata.in_reply_to,
                        &message.metadata.references,
                        &message.metadata.from_addr,
                        &message.metadata.subject,
                        &inbound_body,
                        &inbound_body_truncated,
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
                    outbound_body = $4, outbound_body_redacted = $5, outbound_message_id = $6,
                    outbound_reason = $7,
                    updated_at = now()
                WHERE mailbox_id = $8 AND uid_validity = $9 AND uid = $10",
                &[
                    &outbound_action_value(&action.kind),
                    &action.recipients,
                    &empty_string_as_none(&action.subject),
                    &body,
                    &body_redacted,
                    &action.message_id,
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

    async fn ensure_default_classification_policy(
        &self,
        config: &AppConfig,
    ) -> Result<(), StorageError> {
        for (name, description) in default_categories() {
            self.client
                .execute(
                    "INSERT INTO email_categories (name, description, source)
                    VALUES ($1, $2, 'seed')
                    ON CONFLICT (name) DO UPDATE
                    SET description = EXCLUDED.description,
                        status = 'active',
                        updated_at = now()",
                    &[&name, &description],
                )
                .await?;
        }
        for (name, description) in default_topics() {
            self.client
                .execute(
                    "INSERT INTO email_topics (name, description, source)
                    VALUES ($1, $2, 'seed')
                    ON CONFLICT (name) DO UPDATE
                    SET description = EXCLUDED.description,
                        status = 'active',
                        updated_at = now()",
                    &[&name, &description],
                )
                .await?;
        }

        let category_row = self
            .client
            .query_one(
                "SELECT id FROM email_categories WHERE name = 'marketing_vendor'",
                &[],
            )
            .await?;
        let category_id: i64 = category_row.get(0);
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            for mailbox in &config.mailboxes {
                self.client
                    .execute(
                        "INSERT INTO email_rules
                    (mailbox_id, name, category_id, action, reply_goal, enabled, priority)
                    SELECT $1, $2, $3, $4, $5, TRUE, 100
                    WHERE NOT EXISTS (
                        SELECT 1 FROM email_rules WHERE mailbox_id = $1 AND category_id = $3
                    )",
                        &[
                            &mailbox.id,
                            &"Auto-decline marketing/vendor outreach",
                            &category_id,
                            &EmailRuleAction::Reply.as_str(),
                            &DEFAULT_MARKETING_REPLY_GOAL,
                        ],
                    )
                    .await?;
                self.client
                    .execute(
                        "INSERT INTO email_rule_mailbox_seeds (mailbox_id)
                    VALUES ($1)
                    ON CONFLICT DO NOTHING",
                        &[&mailbox.id],
                    )
                    .await?;
            }
            Ok(())
        }
        .await;
        self.finish_transaction(result).await
    }

    async fn active_email_taxonomy(&self) -> Result<EmailTaxonomy, StorageError> {
        let categories = self
            .client
            .query(
                "SELECT id, name, description, status, source, created_at::text, updated_at::text
                FROM email_categories
                WHERE status = 'active'
                ORDER BY name ASC",
                &[],
            )
            .await?
            .into_iter()
            .map(email_category_from_row)
            .collect();
        let topics = self
            .client
            .query(
                "SELECT id, name, description, status, source, created_at::text, updated_at::text
                FROM email_topics
                WHERE status = 'active'
                ORDER BY name ASC",
                &[],
            )
            .await?
            .into_iter()
            .map(email_topic_from_row)
            .collect();
        Ok(EmailTaxonomy { categories, topics })
    }

    async fn resolve_email_classification(
        &self,
        classification: &EmailClassification,
    ) -> Result<ResolvedEmailClassification, StorageError> {
        let category_name = normalize_label_name(&classification.category);
        let category_row = self
            .client
            .query_one(
                "INSERT INTO email_categories (name, description, source)
                VALUES ($1, 'AI-created category', 'ai')
                ON CONFLICT (name) DO UPDATE
                SET status = 'active', updated_at = now()
                RETURNING id, name",
                &[&category_name],
            )
            .await?;
        let category_id: i64 = category_row.get(0);
        let category: String = category_row.get(1);

        let topic_names = if classification.topics.is_empty() {
            vec!["general".to_string()]
        } else {
            classification
                .topics
                .iter()
                .map(|topic| normalize_label_name(topic))
                .collect::<Vec<_>>()
        };
        let mut topic_ids = Vec::new();
        let mut topics = Vec::new();
        for topic_name in topic_names {
            let topic_row = self
                .client
                .query_one(
                    "INSERT INTO email_topics (name, description, source)
                    VALUES ($1, 'AI-created topic', 'ai')
                    ON CONFLICT (name) DO UPDATE
                    SET status = 'active', updated_at = now()
                    RETURNING id, name",
                    &[&topic_name],
                )
                .await?;
            let topic_id: i64 = topic_row.get(0);
            let topic: String = topic_row.get(1);
            if !topic_ids.contains(&topic_id) {
                topic_ids.push(topic_id);
                topics.push(topic);
            }
        }

        Ok(ResolvedEmailClassification {
            category_id,
            category,
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
        let rows = self
            .client
            .query(
                "SELECT r.id, r.mailbox_id, r.name, r.category_id, c.name,
                    COALESCE(array_remove(array_agg(rt.topic_id ORDER BY t.name), NULL), '{}'),
                    COALESCE(array_remove(array_agg(t.name ORDER BY t.name), NULL), '{}'),
                    r.action, r.reply_goal, r.enabled, r.priority, r.created_at::text, r.updated_at::text
                FROM email_rules r
                JOIN email_categories c ON c.id = r.category_id
                LEFT JOIN email_rule_topics rt ON rt.rule_id = r.id
                LEFT JOIN email_topics t ON t.id = rt.topic_id
                WHERE r.mailbox_id = $1 AND r.category_id = $2 AND r.enabled = TRUE
                GROUP BY r.id, c.name
                ORDER BY CASE WHEN COUNT(rt.topic_id) = 0 THEN 1 ELSE 0 END, r.priority ASC, r.id ASC",
                &[&mailbox_id, &classification.category_id],
            )
            .await?;
        for row in rows {
            let rule = email_rule_from_row(row)?;
            if rule.topic_ids.is_empty()
                || rule
                    .topic_ids
                    .iter()
                    .any(|topic_id| classification.topic_ids.contains(topic_id))
            {
                return Ok(Some(rule));
            }
        }
        Ok(None)
    }

    async fn record_email_classification(
        &self,
        key: &DedupeKey,
        classification: &ResolvedEmailClassification,
        decision_source: &str,
        matched_rule: Option<&EmailRule>,
    ) -> Result<(), StorageError> {
        let confidence = classification.confidence as i16;
        let matched_rule_id = matched_rule.map(|rule| rule.id);
        let matched_rule_name = matched_rule.map(|rule| rule.name.clone());
        let matched_rule_goal = matched_rule.map(|rule| rule.reply_goal.clone());
        self.client
            .execute(
                "UPDATE processing_runs
                SET classification_category_id = $1,
                    classification_topic_ids = $2,
                    classification_reason = $3,
                    classification_confidence = $4,
                    decision_source = $5,
                    matched_rule_id = $6,
                    matched_rule_name = $7,
                    matched_rule_goal = $8,
                    updated_at = now()
                WHERE mailbox_id = $9 AND uid_validity = $10 AND uid = $11",
                &[
                    &classification.category_id,
                    &classification.topic_ids,
                    &classification.reason,
                    &confidence,
                    &decision_source,
                    &matched_rule_id,
                    &matched_rule_name,
                    &matched_rule_goal,
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
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::{
        AgentConfig, AiConfig, AiProtocol, ImapConfig, LoggingConfig, MailboxConfig, PromptConfig,
        ReviewConfig, SmtpConfig,
    };
    use crate::logging::LogLevel;
    use crate::mail::MessageMetadata;
    use crate::storage::{EMAIL_CLASSIFICATION_RULES_SQL, HISTORY_BODY_THREADING_SQL, INIT_SQL};

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
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].get::<_, i32>(0), 1);
        assert_eq!(rows[0].get::<_, String>(1), "001_init");
        assert_eq!(rows[0].get::<_, String>(2), migration_checksum(INIT_SQL));
        assert_eq!(rows[1].get::<_, i32>(0), 2);
        assert_eq!(rows[1].get::<_, String>(1), "002_history_body_threading");
        assert_eq!(
            rows[1].get::<_, String>(2),
            migration_checksum(HISTORY_BODY_THREADING_SQL)
        );
        assert_eq!(rows[2].get::<_, i32>(0), 3);
        assert_eq!(
            rows[2].get::<_, String>(1),
            "003_email_classification_rules"
        );
        assert_eq!(
            rows[2].get::<_, String>(2),
            migration_checksum(EMAIL_CLASSIFICATION_RULES_SQL)
        );

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
            message_id: Some("<reply-71@example.com>".to_string()),
            in_reply_to: Some("<71@example.com>".to_string()),
            references: vec!["<71@example.com>".to_string()],
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
        let category = pg
            .store
            .create_email_category("question", "A project question")
            .await
            .unwrap();
        let topic = pg
            .store
            .create_email_topic("dense_mem", "Dense-Mem")
            .await
            .unwrap();
        let rule = pg
            .store
            .create_email_rule(NewEmailRule {
                mailbox_id: "support".to_string(),
                name: "Answer Dense-Mem questions".to_string(),
                category_id: category.id,
                topic_ids: vec![topic.id],
                action: EmailRuleAction::Reply,
                reply_goal: "Answer using project context.".to_string(),
                enabled: true,
                priority: 20,
            })
            .await
            .unwrap();
        pg.store
            .record_email_classification(
                &key,
                &ResolvedEmailClassification {
                    category_id: category.id,
                    category: category.name.clone(),
                    topic_ids: vec![topic.id],
                    topics: vec![topic.name.clone()],
                    reason: "asks about Dense-Mem".to_string(),
                    confidence: 88,
                },
                "rule",
                Some(&rule),
            )
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
        assert_eq!(email.thread_id, "<71@example.com>");
        assert_eq!(email.message_id, Some("<71@example.com>".to_string()));
        assert_eq!(email.in_reply_to, None);
        assert_eq!(email.references, Vec::<String>::new());
        assert_eq!(email.from_addr, "person@example.com");
        assert_eq!(email.subject, "Question");
        assert_eq!(email.inbound_body, Some("Body".to_string()));
        assert!(!email.inbound_body_truncated);
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
        assert_eq!(
            email.outbound_message_id,
            Some("<reply-71@example.com>".to_string())
        );
        assert_eq!(email.outbound_reason, Some("known answer".to_string()));
        assert_eq!(email.outbound_review_status, Some("approved".to_string()));
        assert_eq!(email.outbound_review_reason, Some("looks safe".to_string()));
        assert_eq!(email.classification_category, Some("question".to_string()));
        assert_eq!(email.classification_topics, vec!["dense_mem"]);
        assert_eq!(
            email.classification_reason,
            Some("asks about Dense-Mem".to_string())
        );
        assert_eq!(email.classification_confidence, Some(88));
        assert_eq!(email.decision_source, Some("rule".to_string()));
        assert_eq!(email.matched_rule_id, Some(rule.id));
        assert_eq!(
            email.matched_rule_name,
            Some("Answer Dense-Mem questions".to_string())
        );
        assert_eq!(
            email.matched_rule_goal,
            Some("Answer using project context.".to_string())
        );
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
            message_id: None,
            in_reply_to: None,
            references: vec![],
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

    #[tokio::test]
    async fn pg_store_links_reply_to_generated_outbound_message_id() {
        let Some(pg) = TestPgStore::create().await else {
            return;
        };
        pg.store.migrate().await.unwrap();

        let original_run_id = uuid::Uuid::new_v4().to_string();
        let original = message(73);
        let original_key = original.metadata.dedupe_key();
        let reply = OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            reason: "known answer".to_string(),
            message_id: Some("<auto-reply@example.com>".to_string()),
            in_reply_to: original.metadata.message_id.clone(),
            references: vec![original.metadata.message_id.clone().unwrap()],
        };
        pg.store
            .claim_message(&original_run_id, &original)
            .await
            .unwrap();
        pg.store
            .record_outbound_action(&original_key, &reply)
            .await
            .unwrap();

        let mut follow_up = message(74);
        follow_up.metadata.message_id = Some("<follow-up@example.com>".to_string());
        follow_up.metadata.in_reply_to = Some("<auto-reply@example.com>".to_string());
        follow_up.metadata.references = vec![];
        pg.store
            .claim_message(&uuid::Uuid::new_v4().to_string(), &follow_up)
            .await
            .unwrap();

        let emails = pg.store.list_processed_emails(10).await.unwrap();
        let original = emails
            .iter()
            .find(|email| email.uid == 73)
            .expect("original row");
        let follow_up = emails
            .iter()
            .find(|email| email.uid == 74)
            .expect("follow-up row");
        assert_eq!(follow_up.thread_id, original.thread_id);
        assert_eq!(
            follow_up.in_reply_to,
            Some("<auto-reply@example.com>".to_string())
        );

        pg.cleanup().await;
    }

    #[tokio::test]
    async fn pg_store_recovers_default_rule_when_seed_marker_exists_without_rule() {
        let Some(pg) = TestPgStore::create().await else {
            return;
        };
        pg.store.migrate().await.unwrap();
        let config = app_config_with_mailboxes(vec!["support"]);

        pg.store
            .ensure_default_classification_policy(&config)
            .await
            .unwrap();
        pg.store
            .client
            .execute(
                "DELETE FROM email_rules WHERE mailbox_id = $1",
                &[&"support"],
            )
            .await
            .unwrap();
        let marker_count = pg
            .store
            .client
            .query_one(
                "SELECT count(*) FROM email_rule_mailbox_seeds WHERE mailbox_id = $1",
                &[&"support"],
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(marker_count, 1);

        pg.store
            .ensure_default_classification_policy(&config)
            .await
            .unwrap();

        let rule_count = pg
            .store
            .client
            .query_one(
                "SELECT count(*)
                FROM email_rules r
                JOIN email_categories c ON c.id = r.category_id
                WHERE r.mailbox_id = $1 AND c.name = 'marketing_vendor'",
                &[&"support"],
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(rule_count, 1);

        pg.cleanup().await;
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

    fn app_config_with_mailboxes(mailbox_ids: Vec<&str>) -> AppConfig {
        AppConfig {
            version: 1,
            database: DatabaseConfig {
                host: "postgres".to_string(),
                port: 5432,
                username: "user".to_string(),
                password: "db-secret".to_string(),
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
                email_classifier: "classifier.md".into(),
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
            mcp_servers: BTreeMap::new(),
            mailboxes: mailbox_ids
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
                        default_forward_to: vec![],
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
                .execute(
                    "SELECT pg_terminate_backend(pid)
                    FROM pg_stat_activity
                    WHERE datname = $1 AND pid <> pg_backend_pid()",
                    &[&database_name],
                )
                .await
                .unwrap();
            admin
                .batch_execute(&format!(
                    "DROP DATABASE IF EXISTS {}",
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
