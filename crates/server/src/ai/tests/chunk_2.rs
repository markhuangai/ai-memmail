use super::*;

#[tokio::test]
async fn live_decision_engine_classifies_with_configured_taxonomy() {
    let mut config = app_config();
    config.prompts.root = write_prompt_root("classifier-provider");
    let mailbox = mailbox_config();
    let chat = FakeChatProvider::new(vec![
        r#"{"category":"marketing_vendor","topics":["general"],"reason":"offers paid PR","confidence":93}"#,
    ]);
    let engine =
        LiveDecisionEngine::new(chat.clone(), FakeMcpContextProvider::new("unused context"));
    let message = inbound(
        "Re: Paid PR",
        "No thanks.\n\nOn Monday, Vendor wrote:\n> We can get you coverage.",
    );
    let thread_context = ThreadContext {
        thread_id: "<root@example.com>".to_string(),
        messages: vec![crate::mail::ThreadMessage {
            direction: crate::mail::MessageDirection::Outbound,
            message_id: Some("<root@example.com>".to_string()),
            in_reply_to: None,
            references: vec![],
            from_addr: "support@example.com".to_string(),
            recipients: vec!["vendor@example.com".to_string()],
            subject: "Paid PR".to_string(),
            authored_text: "Are you offering paid coverage?".to_string(),
            body_truncated: false,
            timestamp: 1,
        }],
    };

    let classification = engine
        .classify_email(
            &config,
            &mailbox,
            &message,
            &thread_context,
            &EmailTaxonomy {
                categories: vec![crate::classification::EmailCategory {
                    id: 1,
                    name: "marketing_vendor".to_string(),
                    description: "Paid vendor outreach".to_string(),
                    status: "active".to_string(),
                    source: "seed".to_string(),
                    created_at: "test".to_string(),
                    updated_at: "test".to_string(),
                }],
                topics: vec![crate::classification::EmailTopic {
                    id: 1,
                    name: "general".to_string(),
                    description: "General topic".to_string(),
                    status: "active".to_string(),
                    source: "seed".to_string(),
                    created_at: "test".to_string(),
                    updated_at: "test".to_string(),
                }],
            },
        )
        .await
        .unwrap();

    assert_eq!(classification.category, "marketing_vendor");
    let requests = chat.requests();
    assert_eq!(requests.len(), 1);
    let payload = requests[0][1]["content"].as_str().unwrap();
    let payload_json: Value = serde_json::from_str(payload).unwrap();
    assert!(payload.contains("marketing_vendor"));
    assert!(payload.contains("untrusted_email"));
    assert!(payload.contains("Are you offering paid coverage?"));
    assert_eq!(payload_json["untrusted_email"]["plain_text"], "No thanks.");
}

#[tokio::test]
async fn live_decision_engine_uses_rule_prompt_and_mcp_context() {
    let mut config = app_config();
    config.prompts.root = write_prompt_root("rule-provider");
    let mailbox = mailbox_config();
    let chat = FakeChatProvider::new(vec![
        r#"{
            "subject":"Re: Paid PR",
            "body":"Thanks for reaching out, but I am not interested in paid PR services.",
            "reason":"matched marketing rule",
            "safety_notes":"safe decline"
        }"#,
    ]);
    let mcp = FakeMcpContextProvider::new("public context: prefers organic growth");
    let engine = LiveDecisionEngine::new(chat.clone(), mcp.clone());

    let decision = engine
        .rule_decision(
            &config,
            &mailbox,
            &inbound("Paid PR", "We can get you coverage."),
            &ThreadContext::empty("thread".to_string()),
            &EmailClassification {
                category: "marketing_vendor".to_string(),
                topics: vec!["general".to_string()],
                reason: "offers paid PR".to_string(),
                confidence: 93,
            },
            &email_rule(crate::classification::EmailRuleAction::Reply),
        )
        .await
        .unwrap();

    assert_eq!(decision.action.kind, OutboundActionKind::Reply);
    assert!(decision.action.body.contains("not interested in paid PR"));
    assert_eq!(mcp.call_count(), 1);
    let payload = chat.requests()[0][1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(payload.contains("public context: prefers organic growth"));
    assert!(payload.contains("matched_rule"));
}

#[tokio::test]
async fn live_decision_engine_uses_review_prompt_for_outbound_review() {
    let mut config = app_config();
    config.prompts.root = write_prompt_root("review-provider");
    let mailbox = mailbox_config();
    let chat = FakeChatProvider::new(vec![
        r#"{"approved":false,"reason":"unexpected recipient"}"#,
    ]);
    let engine =
        LiveDecisionEngine::new(chat.clone(), FakeMcpContextProvider::new("unused context"));
    let decision = AgentDecision {
        action: crate::mail::OutboundAction {
            kind: crate::mail::OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Question".to_string(),
            body: "Answer".to_string(),
            reason: "draft".to_string(),
            message_id: None,
            in_reply_to: None,
            references: vec![],
        },
        safety_notes: "safe".to_string(),
    };

    let review = engine
        .outbound_review(
            &config,
            &mailbox,
            &inbound("Question", "What coverage?"),
            &ThreadContext::empty("thread".to_string()),
            &decision,
        )
        .await
        .unwrap();

    assert!(!review.approved);
    let requests = chat.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0][0]["role"], "system");
    let payload = requests[0][1]["content"].as_str().unwrap();
    assert!(payload.contains("proposed_action"));
    assert!(payload.contains("untrusted_email"));
}

#[test]
fn recall_without_mcp_returns_explicit_empty_context() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut config = app_config();
    let mut mailbox = mailbox_config();
    mailbox.mcp_servers.clear();
    config.mailboxes = vec![mailbox.clone()];
    let context = runtime
        .block_on(recall_mailbox_context(
            &config,
            &mailbox,
            &inbound("Question", "Body"),
        ))
        .unwrap();
    assert!(context.contains("No MCP servers configured"));
}
