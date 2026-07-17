impl PgStore {
    pub async fn thread_handoff_source(
        &self,
        run_id: &str,
    ) -> Result<ThreadHandoffSource, StorageError> {
        let run_id = parse_run_id(run_id)?;
        let row = self
            .client
            .query_opt(
                "SELECT run_id, mailbox_id, uid_validity, uid,
                    COALESCE(thread_id, message_id, mailbox_id || ':' || uid_validity::text || ':' || uid::text),
                    subject, status, safety_category, inbound_body_truncated
                FROM processing_runs
                WHERE run_id = $1",
                &[&run_id],
            )
            .await?;
        let row = row.ok_or_else(|| StorageError::HandoffSourceNotFound(run_id.to_string()))?;
        Ok(ThreadHandoffSource {
            run_id: row.get(0),
            mailbox_id: row.get(1),
            uid_validity: row.get::<_, i64>(2) as u64,
            uid: row.get::<_, i64>(3) as u64,
            thread_id: row.get(4),
            subject: row.get(5),
            status: row.get(6),
            safety_category: row.get(7),
            inbound_body_truncated: row.get(8),
        })
    }

    pub async fn validate_thread_handoff_ready(
        &self,
        mailbox_id: &str,
        thread_id: &str,
    ) -> Result<(), StorageError> {
        let row = self
            .client
            .query_one(
                "SELECT
                    count(*)::BIGINT,
                    count(*) FILTER (
                        WHERE safety_category IS DISTINCT FROM 'safe'
                            OR inbound_body IS NULL
                            OR inbound_body_truncated
                    )::BIGINT
                FROM processing_runs
                WHERE mailbox_id = $1 AND thread_id = $2",
                &[&mailbox_id, &thread_id],
            )
            .await?;
        let message_count: i64 = row.get(0);
        let blocked_count: i64 = row.get(1);
        if message_count == 0 {
            return Err(StorageError::InvalidHandoff(
                "thread has no stored messages".to_string(),
            ));
        }
        if blocked_count > 0 {
            return Err(StorageError::InvalidHandoff(
                "thread contains unsafe or incomplete stored messages".to_string(),
            ));
        }

        let sent_truncated_count = self
            .client
            .query_one(
                "SELECT count(*)::BIGINT
                FROM sent_messages
                WHERE mailbox_id = $1 AND thread_id = $2 AND body_truncated",
                &[&mailbox_id, &thread_id],
            )
            .await?
            .get::<_, i64>(0);
        if sent_truncated_count > 0 {
            return Err(StorageError::InvalidHandoff(
                "thread contains truncated sent-message history".to_string(),
            ));
        }
        Ok(())
    }

    pub async fn latest_thread_remote_target(
        &self,
        mailbox_id: &str,
        thread_id: &str,
    ) -> Result<String, StorageError> {
        let row = self
            .client
            .query_opt(
                "SELECT from_addr
                FROM processing_runs
                WHERE mailbox_id = $1 AND thread_id = $2 AND safety_category = 'safe'
                ORDER BY created_at DESC, uid DESC
                LIMIT 1",
                &[&mailbox_id, &thread_id],
            )
            .await?;
        row.map(|row| row.get(0))
            .ok_or_else(|| StorageError::InvalidHandoff("thread has no safe inbound sender".to_string()))
    }

    async fn active_thread_handoff_impl(
        &self,
        mailbox_id: &str,
        thread_id: &str,
    ) -> Result<Option<ThreadHandoff>, StorageError> {
        let row = self
            .client
            .query_opt(
                "SELECT mailbox_id, thread_id, destination, remote_target, state, last_error,
                    updated_at::text
                FROM thread_handoffs
                WHERE mailbox_id = $1
                    AND thread_id = $2
                    AND state IN ('active', 'sending', 'uncertain')",
                &[&mailbox_id, &thread_id],
            )
            .await?;
        Ok(row.map(thread_handoff_from_row))
    }

    async fn begin_thread_handoff_delivery_impl(
        &self,
        delivery: &NewThreadHandoffDelivery,
    ) -> Result<ThreadHandoffDelivery, StorageError> {
        self.client.batch_execute("BEGIN").await?;
        let result: Result<ThreadHandoffDelivery, StorageError> = async {
            let inserted = self
                .client
                .query_opt(
                    "INSERT INTO thread_handoff_deliveries
                    (request_id, mailbox_id, thread_id, source_run_id, destination,
                        remote_target, outbound_message_id, status)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, 'sending')
                    ON CONFLICT (mailbox_id, thread_id, request_id) DO NOTHING
                    RETURNING request_id, mailbox_id, thread_id, source_run_id, destination,
                        remote_target, outbound_message_id, status, error",
                    &[
                        &delivery.request_id,
                        &delivery.mailbox_id,
                        &delivery.thread_id,
                        &delivery.source_run_id,
                        &delivery.destination,
                        &delivery.remote_target,
                        &delivery.outbound_message_id,
                    ],
                )
                .await?;
            let stored = match inserted {
                Some(row) => thread_handoff_delivery_from_row(row),
                None => self
                    .thread_handoff_delivery(
                        &delivery.mailbox_id,
                        &delivery.thread_id,
                        delivery.request_id,
                    )
                    .await?,
            };
            self.client
                .execute(
                    "INSERT INTO thread_handoffs
                    (mailbox_id, thread_id, destination, remote_target, state)
                    VALUES ($1, $2, $3, $4, 'sending')
                    ON CONFLICT (mailbox_id, thread_id) DO UPDATE
                    SET state = CASE
                            WHEN thread_handoffs.state = 'active' THEN thread_handoffs.state
                            ELSE 'sending'
                        END,
                        destination = CASE
                            WHEN thread_handoffs.state = 'active' THEN thread_handoffs.destination
                            ELSE EXCLUDED.destination
                        END,
                        remote_target = CASE
                            WHEN thread_handoffs.state = 'active' THEN thread_handoffs.remote_target
                            ELSE EXCLUDED.remote_target
                        END,
                        updated_at = now()",
                    &[
                        &delivery.mailbox_id,
                        &delivery.thread_id,
                        &delivery.destination,
                        &delivery.remote_target,
                    ],
                )
                .await?;
            Ok(stored)
        }
        .await;
        self.finish_transaction(result).await
    }

    async fn finish_thread_handoff_delivery_impl(
        &self,
        mailbox_id: &str,
        thread_id: &str,
        request_id: uuid::Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), StorageError> {
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            self.client
                .execute(
                    "UPDATE thread_handoff_deliveries
                    SET status = $1, error = $2, updated_at = now()
                    WHERE mailbox_id = $3 AND thread_id = $4 AND request_id = $5",
                    &[&status, &error, &mailbox_id, &thread_id, &request_id],
                )
                .await?;
            if status == "sent" {
                self.client
                    .execute(
                        "INSERT INTO thread_handoffs
                        (mailbox_id, thread_id, destination, remote_target, state, last_error)
                        SELECT mailbox_id, thread_id, destination, remote_target, 'active', NULL
                        FROM thread_handoff_deliveries
                        WHERE mailbox_id = $1 AND thread_id = $2 AND request_id = $3
                        ON CONFLICT (mailbox_id, thread_id) DO UPDATE
                        SET destination = EXCLUDED.destination,
                            remote_target = EXCLUDED.remote_target,
                            state = 'active',
                            last_error = NULL,
                            updated_at = now()",
                        &[&mailbox_id, &thread_id, &request_id],
                    )
                    .await?;
            } else {
                self.client
                    .execute(
                        "UPDATE thread_handoffs
                        SET state = CASE WHEN state = 'active' THEN state ELSE 'uncertain' END,
                            last_error = $1,
                            updated_at = now()
                        WHERE mailbox_id = $2 AND thread_id = $3",
                        &[&error, &mailbox_id, &thread_id],
                    )
                    .await?;
            }
            Ok(())
        }
        .await;
        self.finish_transaction(result).await
    }

    async fn thread_handoff_delivery(
        &self,
        mailbox_id: &str,
        thread_id: &str,
        request_id: uuid::Uuid,
    ) -> Result<ThreadHandoffDelivery, StorageError> {
        let row = self
            .client
            .query_one(
                "SELECT request_id, mailbox_id, thread_id, source_run_id, destination,
                    remote_target, outbound_message_id, status, error
                FROM thread_handoff_deliveries
                WHERE mailbox_id = $1 AND thread_id = $2 AND request_id = $3",
                &[&mailbox_id, &thread_id, &request_id],
            )
            .await?;
        Ok(thread_handoff_delivery_from_row(row))
    }
}

fn thread_handoff_from_row(row: tokio_postgres::Row) -> ThreadHandoff {
    ThreadHandoff {
        mailbox_id: row.get(0),
        thread_id: row.get(1),
        destination: row.get(2),
        remote_target: row.get(3),
        state: row.get(4),
        last_error: row.get(5),
        updated_at: row.get(6),
    }
}

fn thread_handoff_delivery_from_row(row: tokio_postgres::Row) -> ThreadHandoffDelivery {
    ThreadHandoffDelivery {
        request_id: row.get(0),
        mailbox_id: row.get(1),
        thread_id: row.get(2),
        source_run_id: row.get(3),
        destination: row.get(4),
        remote_target: row.get(5),
        outbound_message_id: row.get(6),
        status: row.get(7),
        error: row.get(8),
    }
}
