use std::collections::BTreeMap;

use super::*;
use crate::config::{
    AgentConfig, AiConfig, AiProtocol, ImapConfig, LoggingConfig, MailboxConfig, PromptConfig,
    ReviewConfig, SmtpConfig,
};
use crate::logging::LogLevel;
use crate::mail::{MessageMetadata, SentMessage};
use crate::storage::{
    DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL, EMAIL_CLASSIFICATION_RULES_SQL,
    HISTORY_BODY_THREADING_SQL, INIT_SQL, SENT_THREAD_CONTEXT_SQL,
};

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
    assert_eq!(rows.len(), 5);
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
    assert_eq!(rows[3].get::<_, i32>(0), 4);
    assert_eq!(
        rows[3].get::<_, String>(1),
        "004_default_email_rule_seed_uniqueness"
    );
    assert_eq!(
        rows[3].get::<_, String>(2),
        migration_checksum(DEFAULT_EMAIL_RULE_SEED_UNIQUENESS_SQL)
    );
    assert_eq!(rows[4].get::<_, i32>(0), 5);
    assert_eq!(rows[4].get::<_, String>(1), "005_sent_thread_context");
    assert_eq!(
        rows[4].get::<_, String>(2),
        migration_checksum(SENT_THREAD_CONTEXT_SQL)
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

#[tokio::test]
async fn pg_store_concurrent_default_policy_seeding_creates_one_rule() {
    let Some(pg) = TestPgStore::create().await else {
        return;
    };
    pg.store.migrate().await.unwrap();
    let config = app_config_with_mailboxes(vec!["support"]);
    let store_config = pg.store_config();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let config = config.clone();
        let store_config = store_config.clone();
        handles.push(tokio::spawn(async move {
            let store = PgStore::connect(&store_config).await.unwrap();
            store
                .ensure_default_classification_policy(&config)
                .await
                .unwrap();
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    let rule_count = pg
        .store
        .client
        .query_one(
            "SELECT count(*)
            FROM email_rules r
            JOIN email_categories c ON c.id = r.category_id
            WHERE r.mailbox_id = $1
              AND c.name = 'marketing_vendor'
              AND r.name = 'Auto-decline marketing/vendor outreach'",
            &[&"support"],
        )
        .await
        .unwrap()
        .get::<_, i64>(0);
    assert_eq!(rule_count, 1);

    pg.cleanup().await;
}

#[tokio::test]
async fn pg_store_links_manual_sent_message_to_inbound_reply_context() {
    let Some(pg) = TestPgStore::create().await else {
        return;
    };
    pg.store.migrate().await.unwrap();
    let config = app_config_with_mailboxes(vec!["support"]);
    let manual_sent = InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 9,
            uid: 12,
            message_id: Some("<manual-root@example.com>".to_string()),
            in_reply_to: None,
            references: vec![],
            from_addr: "support@example.com".to_string(),
            recipients: vec!["person@example.com".to_string()],
            subject: "Manual question".to_string(),
        },
        plain_text: "Original message sent outside ai-memmail.".to_string(),
    };
    pg.store
        .record_sent_batch(
            "support",
            1_700_000_000,
            &SentFetchBatch {
                folder_name: "Sent".to_string(),
                uid_validity: 9,
                messages: vec![SentMessage {
                    message: manual_sent,
                    internal_date: Some(1_700_000_100),
                }],
                complete: true,
            },
        )
        .await
        .unwrap();
    let mut failed_inbound = message(79);
    failed_inbound.metadata.in_reply_to = Some("<manual-root@example.com>".to_string());
    failed_inbound.metadata.references = vec!["<manual-root@example.com>".to_string()];
    pg.store
        .claim_message(&uuid::Uuid::new_v4().to_string(), &failed_inbound)
        .await
        .unwrap();
    let failed_key = failed_inbound.metadata.dedupe_key();
    pg.store
        .record_safety_result(&failed_key, &SafetyCategory::Safe, "routine")
        .await
        .unwrap();
    pg.store
        .record_outbound_action(
            &failed_key,
            &OutboundAction {
                kind: OutboundActionKind::Reply,
                recipients: vec!["person@example.com".to_string()],
                subject: "Re: Manual question".to_string(),
                body: "This SMTP attempt failed.".to_string(),
                reason: "test failed send".to_string(),
                message_id: Some("<failed-send@example.com>".to_string()),
                in_reply_to: failed_inbound.metadata.message_id.clone(),
                references: failed_inbound.metadata.references.clone(),
            },
        )
        .await
        .unwrap();
    pg.store
        .update_message_status(
            &failed_key,
            PROCESSING_STATUS_SEND_FAILED,
            Some(&OutboundActionKind::Reply),
        )
        .await
        .unwrap();
    let mut reply = message(80);
    reply.metadata.message_id = Some("<reply@example.com>".to_string());
    reply.metadata.in_reply_to = Some("<manual-root@example.com>".to_string());
    reply.metadata.references = vec!["<manual-root@example.com>".to_string()];
    reply.metadata.subject = "Re: Manual question".to_string();
    reply.plain_text = "New answer.\n\nOn Monday, Mark wrote:\n> Original message".to_string();

    assert_eq!(
        pg.store
            .claim_message(&uuid::Uuid::new_v4().to_string(), &reply)
            .await
            .unwrap(),
        ProcessingClaim::Claimed
    );
    let context = pg.store
        .load_thread_context(&config.mailboxes[0], &reply)
        .await
        .unwrap();
    let state = pg.store.sent_sync_state("support").await.unwrap().unwrap();

    assert_eq!(context.thread_id, "<manual-root@example.com>");
    assert_eq!(context.messages.len(), 2);
    assert_eq!(context.messages[0].direction, MessageDirection::Outbound);
    assert_eq!(context.messages[0].message_id.as_deref(), Some("<manual-root@example.com>"));
    assert_eq!(
        context.messages[0].authored_text,
        "Original message sent outside ai-memmail."
    );
    assert!(context
        .messages
        .iter()
        .all(|message| message.message_id.as_deref() != Some("<failed-send@example.com>")));
    assert_eq!(state.cursor.folder_name, "Sent");
    assert_eq!(state.cursor.uid_validity, 9);
    assert_eq!(state.cursor.last_uid, 12);
    assert!(state.initial_backfill_complete);

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
            recipients: vec![],
            subject: "Question".to_string(),
        },
        plain_text: "Body".to_string(),
    }
}
