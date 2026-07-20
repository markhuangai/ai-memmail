#[tokio::test]
async fn pg_store_links_follow_up_to_portal_reply_thread() {
    let Some(pg) = TestPgStore::create().await else {
        return;
    };
    pg.store.migrate().await.unwrap();
    let config = app_config_with_mailboxes(vec!["support"]);
    let original = message(91);
    let key = original.metadata.dedupe_key();
    pg.store
        .claim_message(&uuid::Uuid::new_v4().to_string(), &original)
        .await
        .unwrap();
    pg.store
        .record_safety_result(&key, &SafetyCategory::Safe, "routine")
        .await
        .unwrap();
    let conversation = pg
        .store
        .list_portal_conversations(10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let portal_message_id = "<portal-reply@example.com>".to_string();
    let request_id = uuid::Uuid::new_v4();
    let new_message = NewPortalMessage {
        portal_message_id: uuid::Uuid::new_v4(),
        conversation_id: conversation.conversation_id,
        request_id,
        mailbox_id: "support".to_string(),
        thread_id: conversation.thread_id.clone(),
        action: "reply".to_string(),
        to_recipients: vec!["person@example.com".to_string()],
        cc_recipients: vec![],
        bcc_recipients: vec![],
        subject: "Re: Question".to_string(),
        authored_text: "Manual answer".to_string(),
        authored_html: None,
        rendered_text: "Manual answer\n\n---------- Conversation history ---------\nBody".to_string(),
        rendered_html: None,
        quoted_text: "Body".to_string(),
        quoted_html: None,
        message_id: portal_message_id.clone(),
        in_reply_to: original.metadata.message_id.clone(),
        references: vec![original.metadata.message_id.clone().unwrap()],
        reply_target: Some("person@example.com".to_string()),
        source_conversation_id: None,
        child_conversation_id: None,
        unsafe_confirmed: false,
    };
    pg.store.begin_portal_message(&new_message).await.unwrap();
    pg.store
        .finish_portal_message(conversation.conversation_id, request_id, "sent", None)
        .await
        .unwrap();

    let mut follow_up = message(92);
    follow_up.metadata.in_reply_to = Some(portal_message_id.clone());
    follow_up.metadata.references = vec![portal_message_id];
    pg.store
        .claim_message(&uuid::Uuid::new_v4().to_string(), &follow_up)
        .await
        .unwrap();
    let context = pg
        .store
        .load_thread_context(&config.mailboxes[0], &follow_up)
        .await
        .unwrap();

    assert_eq!(context.thread_id, conversation.thread_id);
    assert!(context
        .messages
        .iter()
        .any(|message| message.authored_text == "Manual answer"));

    pg.cleanup().await;
}

#[tokio::test]
async fn pg_store_keeps_forward_replies_in_child_conversation() {
    let Some(pg) = TestPgStore::create().await else {
        return;
    };
    pg.store.migrate().await.unwrap();
    let original = message(93);
    pg.store
        .claim_message(&uuid::Uuid::new_v4().to_string(), &original)
        .await
        .unwrap();
    let source = pg
        .store
        .list_portal_conversations(10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let request_id = uuid::Uuid::new_v4();
    let child_id = request_id;
    let forward_message_id = "<portal-forward@example.com>".to_string();
    pg.store
        .create_child_conversation(
            child_id,
            source.conversation_id,
            "support",
            &forward_message_id,
            "Fwd: Question",
        )
        .await
        .unwrap();
    pg.store
        .begin_portal_message(&NewPortalMessage {
            portal_message_id: uuid::Uuid::new_v4(),
            conversation_id: child_id,
            request_id,
            mailbox_id: "support".to_string(),
            thread_id: forward_message_id.clone(),
            action: "forward".to_string(),
            to_recipients: vec!["reviewer@example.com".to_string()],
            cc_recipients: vec![],
            bcc_recipients: vec![],
            subject: "Fwd: Question".to_string(),
            authored_text: "Please review".to_string(),
            authored_html: None,
            rendered_text: "Please review\n\n---------- Conversation history ---------\nBody".to_string(),
            rendered_html: None,
            quoted_text: "Body".to_string(),
            quoted_html: None,
            message_id: forward_message_id.clone(),
            in_reply_to: None,
            references: vec![],
            reply_target: None,
            source_conversation_id: Some(source.conversation_id),
            child_conversation_id: Some(child_id),
            unsafe_confirmed: false,
        })
        .await
        .unwrap();
    pg.store
        .finish_portal_message(child_id, request_id, "sent", None)
        .await
        .unwrap();

    let mut reviewer_reply = message(94);
    reviewer_reply.metadata.from_addr = "reviewer@example.com".to_string();
    reviewer_reply.metadata.in_reply_to = Some(forward_message_id.clone());
    reviewer_reply.metadata.references = vec![forward_message_id.clone()];
    pg.store
        .claim_message(&uuid::Uuid::new_v4().to_string(), &reviewer_reply)
        .await
        .unwrap();
    let emails = pg.store.list_processed_emails(10).await.unwrap();
    let reply = emails
        .iter()
        .find(|email| email.uid == 94)
        .expect("reviewer reply");
    let source_detail = pg
        .store
        .portal_conversation_detail(source.conversation_id)
        .await
        .unwrap()
        .unwrap();
    let child_detail = pg
        .store
        .portal_conversation_detail(child_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reply.thread_id, forward_message_id);
    assert_ne!(reply.thread_id, source.thread_id);
    assert!(!source_detail
        .messages
        .iter()
        .any(|message| message.kind == "portal_forward"));
    assert!(!source_detail.quote_text.contains("Please review"));
    assert!(child_detail
        .messages
        .iter()
        .any(|message| message.kind == "portal_forward"));
    assert!(child_detail.quote_text.contains("Please review"));

    pg.cleanup().await;
}

#[tokio::test]
async fn pg_store_reuses_child_thread_id_for_forward_retry() {
    let Some(pg) = TestPgStore::create().await else {
        return;
    };
    pg.store.migrate().await.unwrap();
    let original = message(95);
    pg.store
        .claim_message(&uuid::Uuid::new_v4().to_string(), &original)
        .await
        .unwrap();
    let source = pg
        .store
        .list_portal_conversations(10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let request_id = uuid::Uuid::new_v4();
    let child_id = request_id;
    let first_thread_id = "<portal-forward-first@example.com>".to_string();
    let retry_thread_id = "<portal-forward-retry@example.com>".to_string();

    let inserted_thread_id = pg
        .store
        .create_child_conversation(
            child_id,
            source.conversation_id,
            "support",
            &first_thread_id,
            "Fwd: Question",
        )
        .await
        .unwrap();
    let retried_thread_id = pg
        .store
        .create_child_conversation(
            child_id,
            source.conversation_id,
            "support",
            &retry_thread_id,
            "Fwd: Question",
        )
        .await
        .unwrap();
    let (record, inserted) = pg
        .store
        .begin_portal_message(&NewPortalMessage {
            portal_message_id: uuid::Uuid::new_v4(),
            conversation_id: child_id,
            request_id,
            mailbox_id: "support".to_string(),
            thread_id: retried_thread_id.clone(),
            action: "forward".to_string(),
            to_recipients: vec!["reviewer@example.com".to_string()],
            cc_recipients: vec![],
            bcc_recipients: vec![],
            subject: "Fwd: Question".to_string(),
            authored_text: "Please review".to_string(),
            authored_html: None,
            rendered_text: "Please review\n\n---------- Conversation history ---------\nBody".to_string(),
            rendered_html: None,
            quoted_text: "Body".to_string(),
            quoted_html: None,
            message_id: retried_thread_id.clone(),
            in_reply_to: None,
            references: vec![],
            reply_target: None,
            source_conversation_id: Some(source.conversation_id),
            child_conversation_id: Some(child_id),
            unsafe_confirmed: false,
        })
        .await
        .unwrap();
    let child_detail = pg
        .store
        .portal_conversation_detail(child_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(inserted_thread_id, first_thread_id);
    assert_eq!(retried_thread_id, first_thread_id);
    assert!(inserted);
    assert_eq!(record.message_id, first_thread_id);
    assert_eq!(child_detail.conversation.thread_id, first_thread_id);

    pg.cleanup().await;
}
