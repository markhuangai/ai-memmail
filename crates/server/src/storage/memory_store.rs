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

    pub fn email_classification(&self, key: &DedupeKey) -> Option<StoredEmailClassification> {
        self.classification
            .lock()
            .expect("memory classification store poisoned")
            .records
            .get(key)
            .cloned()
    }

    pub fn add_email_rule(&self, mut rule: NewEmailRule) -> EmailRule {
        let mut state = self
            .classification
            .lock()
            .expect("memory classification store poisoned");
        rule.name = rule.name.trim().to_string();
        let id = state.next_rule_id;
        state.next_rule_id += 1;
        let category = state
            .categories
            .iter()
            .find(|category| category.id == rule.category_id)
            .map(|category| category.name.clone())
            .unwrap_or_else(|| "other".to_string());
        let topics = rule
            .topic_ids
            .iter()
            .filter_map(|id| {
                state
                    .topics
                    .iter()
                    .find(|topic| topic.id == *id)
                    .map(|topic| topic.name.clone())
            })
            .collect::<Vec<_>>();
        let stored = EmailRule {
            id,
            mailbox_id: rule.mailbox_id,
            name: rule.name,
            category_id: rule.category_id,
            category,
            topic_ids: rule.topic_ids,
            topics,
            action: rule.action,
            reply_goal: rule.reply_goal,
            enabled: rule.enabled,
            priority: rule.priority,
            created_at: "memory".to_string(),
            updated_at: "memory".to_string(),
        };
        state.rules.push(stored.clone());
        stored
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

    async fn ensure_default_classification_policy(
        &self,
        config: &AppConfig,
    ) -> Result<(), StorageError> {
        let mut state = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let Some(category) = state
            .categories
            .iter()
            .find(|category| category.name == "marketing_vendor")
            .cloned()
        else {
            return Ok(());
        };
        for mailbox in &config.mailboxes {
            if state
                .rules
                .iter()
                .any(|rule| rule.mailbox_id == mailbox.id && rule.category_id == category.id)
            {
                continue;
            }
            let id = state.next_rule_id;
            state.next_rule_id += 1;
            state.rules.push(EmailRule {
                id,
                mailbox_id: mailbox.id.clone(),
                name: "Auto-decline marketing/vendor outreach".to_string(),
                category_id: category.id,
                category: category.name.clone(),
                topic_ids: vec![],
                topics: vec![],
                action: EmailRuleAction::Reply,
                reply_goal: DEFAULT_MARKETING_REPLY_GOAL.to_string(),
                enabled: true,
                priority: 100,
                created_at: "memory".to_string(),
                updated_at: "memory".to_string(),
            });
        }
        Ok(())
    }

    async fn active_email_taxonomy(&self) -> Result<EmailTaxonomy, StorageError> {
        let state = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        Ok(EmailTaxonomy {
            categories: state
                .categories
                .iter()
                .filter(|category| category.status == "active")
                .cloned()
                .collect(),
            topics: state
                .topics
                .iter()
                .filter(|topic| topic.status == "active")
                .cloned()
                .collect(),
        })
    }

    async fn resolve_email_classification(
        &self,
        classification: &EmailClassification,
    ) -> Result<ResolvedEmailClassification, StorageError> {
        let mut state = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let category_name = normalize_label_name(&classification.category);
        let category = match state
            .categories
            .iter()
            .find(|category| category.name == category_name)
            .cloned()
        {
            Some(category) => category,
            None => {
                let category = memory_category(
                    state.next_category_id,
                    &category_name,
                    "AI-created category",
                    "ai",
                );
                state.next_category_id += 1;
                state.categories.push(category.clone());
                category
            }
        };

        let mut topic_ids = Vec::new();
        let mut topics = Vec::new();
        let topic_names = if classification.topics.is_empty() {
            vec!["general".to_string()]
        } else {
            classification
                .topics
                .iter()
                .map(|topic| normalize_label_name(topic))
                .collect::<Vec<_>>()
        };
        for topic_name in topic_names {
            let topic = match state
                .topics
                .iter()
                .find(|topic| topic.name == topic_name)
                .cloned()
            {
                Some(topic) => topic,
                None => {
                    let topic =
                        memory_topic(state.next_topic_id, &topic_name, "AI-created topic", "ai");
                    state.next_topic_id += 1;
                    state.topics.push(topic.clone());
                    topic
                }
            };
            if !topic_ids.contains(&topic.id) {
                topic_ids.push(topic.id);
                topics.push(topic.name);
            }
        }

        Ok(ResolvedEmailClassification {
            category_id: category.id,
            category: category.name,
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
        let mut rules = self
            .classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .rules
            .iter()
            .filter(|rule| {
                rule.enabled
                    && rule.mailbox_id == mailbox_id
                    && rule.category_id == classification.category_id
                    && (rule.topic_ids.is_empty()
                        || rule
                            .topic_ids
                            .iter()
                            .any(|topic_id| classification.topic_ids.contains(topic_id)))
            })
            .cloned()
            .collect::<Vec<_>>();
        rules.sort_by_key(|rule| {
            (
                if rule.topic_ids.is_empty() { 1 } else { 0 },
                rule.priority,
                rule.id,
            )
        });
        Ok(rules.into_iter().next())
    }

    async fn record_email_classification(
        &self,
        key: &DedupeKey,
        classification: &ResolvedEmailClassification,
        decision_source: &str,
        matched_rule: Option<&EmailRule>,
    ) -> Result<(), StorageError> {
        self.classification
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .records
            .insert(
                key.clone(),
                StoredEmailClassification {
                    category: classification.category.clone(),
                    topics: classification.topics.clone(),
                    reason: classification.reason.clone(),
                    confidence: classification.confidence,
                    decision_source: decision_source.to_string(),
                    matched_rule_name: matched_rule.map(|rule| rule.name.clone()),
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
