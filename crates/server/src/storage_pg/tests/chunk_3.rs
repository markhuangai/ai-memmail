#[tokio::test]
async fn pg_store_records_thread_handoff_summary_without_rewriting_history_action() {
    let Some(pg) = TestPgStore::create().await else {
        return;
    };
    pg.store.migrate().await.unwrap();

    let run_id = uuid::Uuid::new_v4().to_string();
    let message = message(82);
    let key = message.metadata.dedupe_key();
    let action = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Question".to_string(),
        body: "Answer".to_string(),
        reason: "known answer".to_string(),
        reply_to: None,
        message_id: Some("<reply-82@example.com>".to_string()),
        in_reply_to: Some("<82@example.com>".to_string()),
        references: vec!["<82@example.com>".to_string()],
    };
    pg.store.claim_message(&run_id, &message).await.unwrap();
    pg.store
        .record_safety_result(&key, &SafetyCategory::Safe, "routine")
        .await
        .unwrap();
    pg.store
        .record_outbound_action(&key, &action)
        .await
        .unwrap();
    pg.store
        .update_message_status(&key, "replied", Some(&OutboundActionKind::Reply))
        .await
        .unwrap();
    let source = pg.store.thread_handoff_source(&run_id).await.unwrap();
    assert_eq!(source.thread_id, message.metadata.message_id.clone().unwrap());
    pg.store
        .validate_thread_handoff_ready(&source.mailbox_id, &source.thread_id)
        .await
        .unwrap();
    assert_eq!(
        pg.store
            .latest_thread_remote_target(&source.mailbox_id, &source.thread_id)
            .await
            .unwrap(),
        "person@example.com"
    );

    let request_id = uuid::Uuid::new_v4();
    let delivery = NewThreadHandoffDelivery {
        request_id,
        mailbox_id: source.mailbox_id.clone(),
        thread_id: source.thread_id.clone(),
        source_run_id: Some(source.run_id),
        destination: "mark.personal@example.com".to_string(),
        remote_target: "person@example.com".to_string(),
        outbound_message_id: "<handoff-82@example.com>".to_string(),
    };
    pg.store
        .begin_thread_handoff_delivery(&delivery)
        .await
        .unwrap();
    pg.store
        .finish_thread_handoff_delivery(
            &source.mailbox_id,
            &source.thread_id,
            request_id,
            "sent",
            None,
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
    assert_eq!(email.outbound_action.as_deref(), Some("reply"));
    assert_eq!(email.status, "replied");
    let handoff = email.handoff.expect("thread handoff summary");
    assert_eq!(handoff.state, "active");
    assert_eq!(handoff.destination, "mark.personal@example.com");
    assert_eq!(handoff.remote_target, "person@example.com");

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
        reply_to: None,
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
                reply_to: None,
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
