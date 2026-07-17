use super::*;

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
    assert!(DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL
        .contains("email_rules_default_marketing_seed_unique_idx"));
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
    assert_eq!(MIGRATIONS[2].version, 3);
    assert_eq!(MIGRATIONS[2].name, "003_email_classification_rules");
    assert_eq!(MIGRATIONS[2].sql, EMAIL_CLASSIFICATION_RULES_SQL);
    assert_eq!(MIGRATIONS[3].version, 4);
    assert_eq!(MIGRATIONS[3].name, "004_default_email_rule_seed_uniqueness");
    assert_eq!(MIGRATIONS[3].sql, DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL);
    assert_eq!(MIGRATIONS[4].version, 5);
    assert_eq!(MIGRATIONS[4].name, "005_sent_thread_context");
    assert_eq!(MIGRATIONS[4].sql, SENT_THREAD_CONTEXT_SQL);
    assert!(SENT_THREAD_CONTEXT_SQL.contains("CREATE TABLE IF NOT EXISTS sent_messages"));
    assert!(SENT_THREAD_CONTEXT_SQL.contains("CREATE TABLE IF NOT EXISTS mailbox_sync_state"));
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
fn processing_stale_window_matches_worker_heartbeat_expectation() {
    assert_eq!(PROCESSING_STALE_AFTER_MINUTES, 3);
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
        reply_to: None,
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
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    assert_eq!(outbound_body_for_storage(&forward), (None, true));

    let noop = OutboundAction {
        kind: OutboundActionKind::Noop,
        recipients: vec![],
        subject: String::new(),
        body: String::new(),
        reason: "nothing to do".to_string(),
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    assert_eq!(outbound_body_for_storage(&noop), (None, false));
}

#[test]
fn inbound_body_storage_caps_large_message_bodies() {
    let mut message = message(7);
    message.plain_text = "a".repeat(INBOUND_BODY_STORAGE_MAX_CHARS + 10);

    let (body, truncated) = inbound_body_for_storage(&message);

    assert_eq!(body.len(), INBOUND_BODY_STORAGE_MAX_CHARS);
    assert!(truncated);

    message.plain_text = "short body".to_string();
    let (body, truncated) = inbound_body_for_storage(&message);
    assert_eq!(body, "short body");
    assert!(!truncated);
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
    assert_eq!(
        safety_category_value(&SafetyCategory::Jailbreak),
        "jailbreak"
    );
    assert_eq!(safety_category_value(&SafetyCategory::Hacking), "hacking");
    assert_eq!(safety_category_value(&SafetyCategory::Unknown), "unknown");
    assert_eq!(safety_category_value(&SafetyCategory::Safe), "safe");
}

#[test]
fn empty_string_as_none_trims_before_deciding() {
    assert_eq!(empty_string_as_none("  "), None);
    assert_eq!(empty_string_as_none(" value "), Some(" value "));
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
        reply_to: None,
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

#[tokio::test]
async fn memory_store_resolves_unknown_labels_and_records_classification() {
    let store = MemoryProcessingStore::default();
    let message = message(46);
    let key = message.metadata.dedupe_key();

    let resolved = store
        .resolve_email_classification(&EmailClassification {
            category: "New Category!".to_string(),
            topics: vec![
                "Dense Mem".to_string(),
                "Dense Mem".to_string(),
                "New Topic".to_string(),
            ],
            reason: "category and topic are model-created".to_string(),
            confidence: 255,
        })
        .await
        .unwrap();

    assert_eq!(resolved.category, "new_category");
    assert_eq!(resolved.topics, vec!["dense_mem", "new_topic"]);
    assert_eq!(resolved.confidence, 100);

    let taxonomy = store.active_email_taxonomy().await.unwrap();
    assert!(taxonomy
        .categories
        .iter()
        .any(|category| category.name == "new_category" && category.source == "ai"));
    assert!(taxonomy
        .topics
        .iter()
        .any(|topic| topic.name == "new_topic" && topic.source == "ai"));

    store
        .record_email_classification(&key, &resolved, "agent", None)
        .await
        .unwrap();

    assert_eq!(
        store.email_classification(&key),
        Some(StoredEmailClassification {
            category: "new_category".to_string(),
            topics: vec!["dense_mem".to_string(), "new_topic".to_string()],
            reason: "category and topic are model-created".to_string(),
            confidence: 100,
            decision_source: "agent".to_string(),
            matched_rule_name: None,
        })
    );
}

#[tokio::test]
async fn memory_store_defaults_empty_topics_to_general() {
    let store = MemoryProcessingStore::default();

    let resolved = store
        .resolve_email_classification(&EmailClassification {
            category: "question".to_string(),
            topics: vec![],
            reason: "asks about setup".to_string(),
            confidence: 87,
        })
        .await
        .unwrap();

    assert_eq!(resolved.category, "question");
    assert_eq!(resolved.topics, vec!["general"]);
    assert_eq!(resolved.confidence, 87);
}

#[tokio::test]
async fn memory_store_matches_topic_specific_rule_before_general_rule() {
    let store = MemoryProcessingStore::default();
    let taxonomy = store.active_email_taxonomy().await.unwrap();
    let question = taxonomy
        .categories
        .iter()
        .find(|category| category.name == "question")
        .unwrap();
    let dense_mem = taxonomy
        .topics
        .iter()
        .find(|topic| topic.name == "dense_mem")
        .unwrap();

    let general = store.add_email_rule(NewEmailRule {
        mailbox_id: "support".to_string(),
        name: "General question rule".to_string(),
        category_id: question.id,
        topic_ids: vec![],
        action: EmailRuleAction::Forward,
        reply_goal: "Forward broad questions to Mark.".to_string(),
        enabled: true,
        priority: 1,
    });
    let topic_specific = store.add_email_rule(NewEmailRule {
        mailbox_id: "support".to_string(),
        name: "Dense-Mem answer rule".to_string(),
        category_id: question.id,
        topic_ids: vec![dense_mem.id],
        action: EmailRuleAction::Reply,
        reply_goal: "Answer Dense-Mem setup questions.".to_string(),
        enabled: true,
        priority: 100,
    });
    store.add_email_rule(NewEmailRule {
        mailbox_id: "support".to_string(),
        name: "Disabled Dense-Mem rule".to_string(),
        category_id: question.id,
        topic_ids: vec![dense_mem.id],
        action: EmailRuleAction::Noop,
        reply_goal: String::new(),
        enabled: false,
        priority: 0,
    });

    let resolved = store
        .resolve_email_classification(&EmailClassification {
            category: "question".to_string(),
            topics: vec!["dense_mem".to_string()],
            reason: "asks about Dense-Mem".to_string(),
            confidence: 91,
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .find_matching_email_rule("support", &resolved)
            .await
            .unwrap(),
        Some(topic_specific.clone())
    );
    assert_eq!(
        store
            .find_matching_email_rule("other", &resolved)
            .await
            .unwrap(),
        None
    );
    assert_ne!(general.id, topic_specific.id);
}

#[tokio::test]
async fn memory_store_seeds_default_marketing_rule_once_per_mailbox() {
    let store = MemoryProcessingStore::default();
    let config = app_config_with_mailboxes(vec!["support", "sales"]);

    store
        .ensure_default_classification_policy(&config)
        .await
        .unwrap();
    store
        .ensure_default_classification_policy(&config)
        .await
        .unwrap();

    let resolved = store
        .resolve_email_classification(&EmailClassification {
            category: "marketing_vendor".to_string(),
            topics: vec!["general".to_string()],
            reason: "offers paid ads".to_string(),
            confidence: 94,
        })
        .await
        .unwrap();

    let support_rule = store
        .find_matching_email_rule("support", &resolved)
        .await
        .unwrap()
        .unwrap();
    let sales_rule = store
        .find_matching_email_rule("sales", &resolved)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(support_rule.name, "Auto-decline marketing/vendor outreach");
    assert_eq!(support_rule.reply_goal, DEFAULT_MARKETING_REPLY_GOAL);
    assert_eq!(sales_rule.name, "Auto-decline marketing/vendor outreach");
    assert_ne!(support_rule.id, sales_rule.id);
}
