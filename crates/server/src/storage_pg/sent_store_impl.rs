impl PgStore {
    async fn sent_sync_state_impl(
        &self,
        mailbox_id: &str,
    ) -> Result<Option<SentSyncState>, StorageError> {
        let row = self
            .client
            .query_opt(
                "SELECT folder_name, uid_validity, last_uid, backfill_cutoff_epoch,
                initial_backfill_complete
            FROM mailbox_sync_state
            WHERE mailbox_id = $1 AND folder_role = 'sent'",
                &[&mailbox_id],
            )
            .await?;
        Ok(row.map(|row| SentSyncState {
            cursor: SentSyncCursor {
                folder_name: row.get(0),
                uid_validity: row.get::<_, i64>(1) as u64,
                last_uid: row.get::<_, i64>(2) as u64,
                backfill_cutoff: row.get(3),
            },
            initial_backfill_complete: row.get(4),
        }))
    }

    async fn record_sent_batch_impl(
        &self,
        mailbox_id: &str,
        backfill_cutoff: i64,
        batch: &SentFetchBatch,
    ) -> Result<(), StorageError> {
        for sent in &batch.messages {
            let thread_id = self.thread_id_for_message(&sent.message).await?;
            let (body, body_truncated) = inbound_body_for_storage(&sent.message);
            self.client
                .execute(
                    "INSERT INTO sent_messages
                    (mailbox_id, folder_name, uid_validity, uid, thread_id, message_id,
                        in_reply_to, message_references, from_addr, recipients, subject,
                        body, body_truncated, internal_date_epoch)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                    ON CONFLICT (mailbox_id, folder_name, uid_validity, uid) DO UPDATE
                    SET thread_id = EXCLUDED.thread_id,
                        message_id = EXCLUDED.message_id,
                        in_reply_to = EXCLUDED.in_reply_to,
                        message_references = EXCLUDED.message_references,
                        from_addr = EXCLUDED.from_addr,
                        recipients = EXCLUDED.recipients,
                        subject = EXCLUDED.subject,
                        body = EXCLUDED.body,
                        body_truncated = EXCLUDED.body_truncated,
                        internal_date_epoch = EXCLUDED.internal_date_epoch,
                        updated_at = now()",
                    &[
                        &mailbox_id,
                        &batch.folder_name,
                        &(batch.uid_validity as i64),
                        &(sent.message.metadata.uid as i64),
                        &thread_id,
                        &sent.message.metadata.message_id,
                        &sent.message.metadata.in_reply_to,
                        &sent.message.metadata.references,
                        &sent.message.metadata.from_addr,
                        &sent.message.metadata.recipients,
                        &sent.message.metadata.subject,
                        &body,
                        &body_truncated,
                        &sent.internal_date,
                    ],
                )
                .await?;
        }
        let last_uid = batch
            .messages
            .iter()
            .map(|sent| sent.message.metadata.uid)
            .max()
            .unwrap_or_default() as i64;
        self.client
            .execute(
                "INSERT INTO mailbox_sync_state
                (mailbox_id, folder_role, folder_name, uid_validity, last_uid,
                    backfill_cutoff_epoch, initial_backfill_complete)
                VALUES ($1, 'sent', $2, $3, $4, $5, $6)
                ON CONFLICT (mailbox_id, folder_role) DO UPDATE
                SET folder_name = EXCLUDED.folder_name,
                    uid_validity = EXCLUDED.uid_validity,
                    last_uid = CASE
                        WHEN mailbox_sync_state.folder_name = EXCLUDED.folder_name
                            AND mailbox_sync_state.uid_validity = EXCLUDED.uid_validity
                        THEN GREATEST(mailbox_sync_state.last_uid, EXCLUDED.last_uid)
                        ELSE EXCLUDED.last_uid
                    END,
                    backfill_cutoff_epoch = CASE
                        WHEN mailbox_sync_state.folder_name = EXCLUDED.folder_name
                            AND mailbox_sync_state.uid_validity = EXCLUDED.uid_validity
                        THEN mailbox_sync_state.backfill_cutoff_epoch
                        ELSE EXCLUDED.backfill_cutoff_epoch
                    END,
                    initial_backfill_complete = CASE
                        WHEN mailbox_sync_state.folder_name = EXCLUDED.folder_name
                            AND mailbox_sync_state.uid_validity = EXCLUDED.uid_validity
                        THEN mailbox_sync_state.initial_backfill_complete
                            OR EXCLUDED.initial_backfill_complete
                        ELSE EXCLUDED.initial_backfill_complete
                    END,
                    updated_at = now()",
                &[
                    &mailbox_id,
                    &batch.folder_name,
                    &(batch.uid_validity as i64),
                    &last_uid,
                    &backfill_cutoff,
                    &batch.complete,
                ],
            )
            .await?;
        Ok(())
    }

    async fn load_thread_context_impl(
        &self,
        mailbox: &MailboxConfig,
        message: &InboundMessage,
    ) -> Result<ThreadContext, StorageError> {
        let thread_id = self.thread_id_for_message(message).await?;
        let key = message.metadata.dedupe_key();
        let rows = self
            .client
            .query(
                "SELECT message_id, in_reply_to, message_references, from_addr,
                inbound_recipients, subject, inbound_body, inbound_body_truncated,
                (EXTRACT(EPOCH FROM created_at))::BIGINT,
                outbound_message_id, outbound_recipients, outbound_subject,
                outbound_body, outbound_body_redacted, outbound_action, status,
                (EXTRACT(EPOCH FROM updated_at))::BIGINT
            FROM processing_runs
            WHERE mailbox_id = $1 AND thread_id = $2
                AND NOT (uid_validity = $3 AND uid = $4)",
                &[
                    &mailbox.id,
                    &thread_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                ],
            )
            .await?;
        let mut messages = Vec::new();
        let mut application_message_ids = HashSet::new();
        for row in rows {
            let message_id: Option<String> = row.get(0);
            if let Some(id) = &message_id {
                application_message_ids.insert(id.clone());
            }
            let references: Vec<String> = row.get(2);
            let inbound_timestamp: i64 = row.get(8);
            if let Some(body) = row.get::<_, Option<String>>(6) {
                messages.push(ThreadMessage {
                    direction: MessageDirection::Inbound,
                    message_id: message_id.clone(),
                    in_reply_to: row.get(1),
                    references: references.clone(),
                    from_addr: row.get(3),
                    recipients: row.get(4),
                    subject: row.get(5),
                    authored_text: body,
                    body_truncated: row.get(7),
                    timestamp: inbound_timestamp,
                });
            }
            let outbound_message_id: Option<String> = row.get(9);
            let outbound_was_sent = row.get::<_, String>(15) == "replied";
            if outbound_was_sent
                && row.get::<_, Option<String>>(14).as_deref() == Some("reply")
            {
                if let (Some(body), false) =
                    (row.get::<_, Option<String>>(12), row.get::<_, bool>(13))
                {
                    if let Some(id) = &outbound_message_id {
                        application_message_ids.insert(id.clone());
                    }
                    let mut outbound_references = references;
                    if let Some(id) = &message_id {
                        outbound_references.push(id.clone());
                    }
                    messages.push(ThreadMessage {
                        direction: MessageDirection::Outbound,
                        message_id: outbound_message_id,
                        in_reply_to: message_id,
                        references: outbound_references,
                        from_addr: mailbox.address.clone(),
                        recipients: row.get(10),
                        subject: row.get::<_, Option<String>>(11).unwrap_or_default(),
                        authored_text: body,
                        body_truncated: false,
                        timestamp: row.get(16),
                    });
                }
            }
        }
        let sent_rows = self
            .client
            .query(
                "SELECT message_id, in_reply_to, message_references, from_addr, recipients,
                subject, body, body_truncated,
                COALESCE(internal_date_epoch, (EXTRACT(EPOCH FROM created_at))::BIGINT)
            FROM sent_messages
            WHERE mailbox_id = $1 AND thread_id = $2",
                &[&mailbox.id, &thread_id],
            )
            .await?;
        for row in sent_rows {
            let message_id: Option<String> = row.get(0);
            if message_id
                .as_ref()
                .is_some_and(|id| application_message_ids.contains(id))
            {
                continue;
            }
            messages.push(ThreadMessage {
                direction: MessageDirection::Outbound,
                message_id,
                in_reply_to: row.get(1),
                references: row.get(2),
                from_addr: row.get(3),
                recipients: row.get(4),
                subject: row.get(5),
                authored_text: row.get(6),
                body_truncated: row.get(7),
                timestamp: row.get(8),
            });
        }
        messages.sort_by_key(|message| message.timestamp);
        for (index, message) in messages.iter_mut().enumerate() {
            message.authored_text =
                extract_authored_text(&message.authored_text, index > 0).authored_text;
        }
        Ok(ThreadContext {
            thread_id,
            messages,
        })
    }
}
