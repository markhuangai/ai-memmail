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
                    pr.outbound_body, pr.outbound_body_html, pr.outbound_body_redacted,
                    pr.outbound_message_id, pr.outbound_reason,
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
                    pr.created_at::text, pr.updated_at::text,
                    th.state, th.destination, th.remote_target, th.last_error, th.updated_at::text
                FROM processing_runs pr
                LEFT JOIN email_categories c ON c.id = pr.classification_category_id
                LEFT JOIN thread_handoffs th
                    ON th.mailbox_id = pr.mailbox_id
                    AND th.thread_id = COALESCE(pr.thread_id, pr.message_id, pr.mailbox_id || ':' || pr.uid_validity::text || ':' || pr.uid::text)
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
                outbound_body_html: row.get(21),
                outbound_body_redacted: row.get(22),
                outbound_message_id: row.get(23),
                outbound_reason: row.get(24),
                outbound_review_status: row.get(25),
                outbound_review_reason: row.get(26),
                classification_category: row.get(27),
                classification_topics: row.get(28),
                classification_reason: row.get(29),
                classification_confidence: row.get::<_, Option<i16>>(30).map(|value| value as u16),
                decision_source: row.get(31),
                matched_rule_id: row.get(32),
                matched_rule_name: row.get(33),
                matched_rule_goal: row.get(34),
                created_at: row.get(35),
                updated_at: row.get(36),
                logs,
                handoff: row.get::<_, Option<String>>(37).map(|state| ThreadHandoffSummary {
                    state,
                    destination: row.get(38),
                    remote_target: row.get(39),
                    last_error: row.get(40),
                    updated_at: row.get(41),
                }),
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
                WHERE mailbox_id = $1
                    AND (message_id = ANY($2) OR outbound_message_id = ANY($2))
                ORDER BY updated_at ASC
                LIMIT 1",
                &[&message.metadata.mailbox_id, &related_ids],
            )
            .await?;
        let thread_id = row.and_then(|row| row.get::<_, Option<String>>(0));
        let thread_id = if thread_id.is_some() {
            thread_id
        } else {
            self.client
                .query_opt(
                    "SELECT thread_id
                    FROM sent_messages
                    WHERE mailbox_id = $1 AND message_id = ANY($2)
                    ORDER BY internal_date_epoch ASC NULLS LAST, uid ASC
                    LIMIT 1",
                    &[&message.metadata.mailbox_id, &related_ids],
                )
                .await?
                .and_then(|row| row.get::<_, Option<String>>(0))
        };
        Ok(thread_id
            .filter(|thread_id| !thread_id.trim().is_empty())
            .unwrap_or_else(|| message.metadata.thread_id()))
    }

}
