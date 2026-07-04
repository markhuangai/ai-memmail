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
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    let decision = AgentDecision {
        action,
        safety_notes: "safe".to_string(),
    };

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
}
