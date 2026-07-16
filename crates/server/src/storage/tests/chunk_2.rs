use super::*;

struct MinimalStore;

#[async_trait::async_trait]
impl ProcessingStore for MinimalStore {
    async fn claim_message(
        &self,
        _run_id: &str,
        _message: &InboundMessage,
    ) -> Result<ProcessingClaim, StorageError> {
        Ok(ProcessingClaim::Claimed)
    }

    async fn update_message_status(
        &self,
        _key: &DedupeKey,
        _status: &str,
        _outbound_action: Option<&OutboundActionKind>,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn record_safety_result(
        &self,
        _key: &DedupeKey,
        _category: &SafetyCategory,
        _reason: &str,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn upsert_sender_review(
        &self,
        _sender: &str,
        _mailbox_id: &str,
        _reason: &str,
    ) -> Result<(), StorageError> {
        Ok(())
    }
}

#[tokio::test]
async fn processing_store_trait_defaults_are_noops() {
    let store = MinimalStore;
    let message = message(47);
    let key = message.metadata.dedupe_key();
    let action = OutboundAction {
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
    let decision = AgentDecision {
        action,
        safety_notes: "safe".to_string(),
    };

    store.touch_processing(&key).await.unwrap();
    store.record_agent_decision(&key, &decision).await.unwrap();
    store
        .record_outbound_action(&key, &decision.action)
        .await
        .unwrap();
    store
        .record_outbound_review(&key, "approved", "safe")
        .await
        .unwrap();
    store
        .ensure_default_classification_policy(&app_config_with_mailboxes(vec!["support"]))
        .await
        .unwrap();

    let taxonomy = store.active_email_taxonomy().await.unwrap();
    assert!(taxonomy
        .categories
        .iter()
        .any(|category| category.name == "question"));

    let resolved = store
        .resolve_email_classification(&EmailClassification {
            category: "question".to_string(),
            topics: vec!["general".to_string()],
            reason: "default implementation".to_string(),
            confidence: 99,
        })
        .await
        .unwrap();
    assert_eq!(resolved.category, "question");

    let fallback = store
        .resolve_email_classification(&EmailClassification {
            category: "not-in-the-taxonomy".to_string(),
            topics: vec![],
            reason: "exercise fallback policy".to_string(),
            confidence: 101,
        })
        .await
        .unwrap();
    assert_eq!(fallback.category, "other");
    assert_eq!(fallback.topics, vec!["general"]);
    assert_eq!(fallback.confidence, 100);

    assert_eq!(
        store
            .find_matching_email_rule("support", &resolved)
            .await
            .unwrap(),
        None
    );
    store
        .record_email_classification(&key, &resolved, "agent", None)
        .await
        .unwrap();

    assert_eq!(store.sent_sync_state("support").await.unwrap(), None);
    store
        .record_sent_batch(
            "support",
            1_700_000_000,
            &crate::mail::SentFetchBatch {
                folder_name: "Sent".to_string(),
                uid_validity: 2,
                messages: vec![],
                complete: true,
            },
        )
        .await
        .unwrap();
    let config = app_config_with_mailboxes(vec!["support"]);
    let context = store
        .load_thread_context(&config.mailboxes[0], &message)
        .await
        .unwrap();
    assert_eq!(context.thread_id, message.metadata.thread_id());
    assert!(context.messages.is_empty());

    assert_eq!(
        store
            .active_thread_handoff("support", "thread-1")
            .await
            .unwrap(),
        None
    );

    let request_id = uuid::Uuid::new_v4();
    let delivery = NewThreadHandoffDelivery {
        request_id,
        mailbox_id: "support".to_string(),
        thread_id: "thread-1".to_string(),
        source_run_id: None,
        destination: "personal@example.com".to_string(),
        remote_target: "person@example.com".to_string(),
        outbound_message_id: "<handoff@example.com>".to_string(),
    };
    let started = store
        .begin_thread_handoff_delivery(&delivery)
        .await
        .unwrap();
    assert_eq!(started.request_id, request_id);
    assert_eq!(started.status, "sending");
    assert_eq!(started.error, None);
    store
        .finish_thread_handoff_delivery("support", "thread-1", request_id, "sent", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn memory_processing_store_records_thread_handoff_delivery() {
    let store = MemoryProcessingStore::default();
    let request_id = uuid::Uuid::new_v4();

    store.set_thread_handoff(ThreadHandoff {
        mailbox_id: "support".to_string(),
        thread_id: "thread-1".to_string(),
        destination: "personal@example.com".to_string(),
        remote_target: "person@example.com".to_string(),
        state: "paused".to_string(),
        last_error: Some("already sent".to_string()),
        updated_at: "2026-07-16T21:00:00Z".to_string(),
    });
    assert_eq!(
        store
            .active_thread_handoff("support", "thread-1")
            .await
            .unwrap(),
        None
    );

    store.set_thread_handoff(ThreadHandoff {
        mailbox_id: "support".to_string(),
        thread_id: "thread-1".to_string(),
        destination: "personal@example.com".to_string(),
        remote_target: "person@example.com".to_string(),
        state: "active".to_string(),
        last_error: None,
        updated_at: "2026-07-16T21:01:00Z".to_string(),
    });
    let handoff = store
        .active_thread_handoff("support", "thread-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handoff.destination, "personal@example.com");
    assert_eq!(handoff.remote_target, "person@example.com");

    let delivery = NewThreadHandoffDelivery {
        request_id,
        mailbox_id: "support".to_string(),
        thread_id: "thread-1".to_string(),
        source_run_id: None,
        destination: "personal@example.com".to_string(),
        remote_target: "person@example.com".to_string(),
        outbound_message_id: "<handoff@example.com>".to_string(),
    };
    let started = store
        .begin_thread_handoff_delivery(&delivery)
        .await
        .unwrap();
    assert_eq!(started.status, "sending");
    assert_eq!(
        store
            .begin_thread_handoff_delivery(&NewThreadHandoffDelivery {
                outbound_message_id: "<different@example.com>".to_string(),
                ..delivery.clone()
            })
            .await
            .unwrap()
            .outbound_message_id,
        "<handoff@example.com>"
    );

    store
        .finish_thread_handoff_delivery(
            "support",
            "thread-1",
            request_id,
            "failed",
            Some("smtp unavailable"),
        )
        .await
        .unwrap();
    let finished = store
        .handoff_delivery("support", "thread-1", request_id)
        .unwrap();
    assert_eq!(finished.status, "failed");
    assert_eq!(finished.error.as_deref(), Some("smtp unavailable"));
}
