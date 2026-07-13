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
                    message_references, from_addr, inbound_recipients, subject, inbound_body,
                    inbound_body_truncated, status)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
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
                    &message.metadata.recipients,
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
                        inbound_recipients = $8, subject = $9, inbound_body = $10,
                        inbound_body_truncated = $11,
                        updated_at = now()
                    WHERE mailbox_id = $12 AND uid_validity = $13 AND uid = $14
                        AND (status IN ($15, $16) OR (status = $2 AND updated_at < now() - make_interval(mins => $17::int)))
                    RETURNING status",
                    &[
                        &run_id,
                        &PROCESSING_STATUS_PROCESSING,
                        &thread_id,
                        &message.metadata.message_id,
                        &message.metadata.in_reply_to,
                        &message.metadata.references,
                        &message.metadata.from_addr,
                        &message.metadata.recipients,
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

    async fn touch_processing(&self, key: &DedupeKey) -> Result<(), StorageError> {
        self.client
            .execute(
                "UPDATE processing_runs
                SET updated_at = now()
                WHERE mailbox_id = $1 AND uid_validity = $2 AND uid = $3 AND status = $4",
                &[
                    &key.mailbox_id,
                    &(key.uid_validity as i64),
                    &(key.uid as i64),
                    &PROCESSING_STATUS_PROCESSING,
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
                    VALUES ($1, $2, $3, $4, $5, TRUE, 100)
                    ON CONFLICT (mailbox_id, category_id, name)
                    WHERE name = 'Auto-decline marketing/vendor outreach'
                    DO NOTHING",
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

    async fn sent_sync_state(
        &self,
        mailbox_id: &str,
    ) -> Result<Option<SentSyncState>, StorageError> {
        self.sent_sync_state_impl(mailbox_id).await
    }

    async fn record_sent_batch(
        &self,
        mailbox_id: &str,
        backfill_cutoff: i64,
        batch: &SentFetchBatch,
    ) -> Result<(), StorageError> {
        self.record_sent_batch_impl(mailbox_id, backfill_cutoff, batch)
            .await
    }

    async fn load_thread_context(
        &self,
        mailbox: &MailboxConfig,
        message: &InboundMessage,
    ) -> Result<ThreadContext, StorageError> {
        self.load_thread_context_impl(mailbox, message).await
    }

}
