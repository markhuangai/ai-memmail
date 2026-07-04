fn failing_safety_decisions() -> FakeDecisionEngine {
    FakeDecisionEngine {
        scan: safe_scan(),
        classification: question_classification(),
        decision: AgentDecision {
            action: reply_action().clone(),
            safety_notes: "tested".to_string(),
        },
        rule_decision: AgentDecision {
            action: reply_action(),
            safety_notes: "rule tested".to_string(),
        },
        review: OutboundReviewDecision {
            approved: true,
            reason: "approved".to_string(),
        },
        fail_safety: true,
        fail_classification: false,
        fail_agent: false,
        fail_rule: false,
        fail_review: false,
        hang_agent: false,
        missing_classifier_prompt: false,
        missing_rule_prompt: false,
        calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
        reviewed_actions: Arc::new(Mutex::new(Vec::new())),
    }
}
fn failing_agent_decisions() -> FakeDecisionEngine {
    FakeDecisionEngine {
        scan: safe_scan(),
        classification: question_classification(),
        decision: AgentDecision {
            action: reply_action().clone(),
            safety_notes: "tested".to_string(),
        },
        rule_decision: AgentDecision {
            action: reply_action(),
            safety_notes: "rule tested".to_string(),
        },
        review: OutboundReviewDecision {
            approved: true,
            reason: "approved".to_string(),
        },
        fail_safety: false,
        fail_classification: false,
        fail_agent: true,
        fail_rule: false,
        fail_review: false,
        hang_agent: false,
        missing_classifier_prompt: false,
        missing_rule_prompt: false,
        calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
        reviewed_actions: Arc::new(Mutex::new(Vec::new())),
    }
}

fn question_classification() -> EmailClassification {
    EmailClassification {
        category: "question".to_string(),
        topics: vec!["general".to_string()],
        reason: "asks a routine question".to_string(),
        confidence: 90,
    }
}

fn marketing_classification() -> EmailClassification {
    EmailClassification {

        category: "marketing_vendor".to_string(),
        topics: vec!["general".to_string()],
        reason: "offers a paid promotion service".to_string(),
        confidence: 94,
    }
}

fn test_taxonomy() -> EmailTaxonomy {
    EmailTaxonomy {
        categories: vec![
            EmailCategory {
                id: 1,
                name: "question".to_string(),
                description: "Question".to_string(),
                status: "active".to_string(),
                source: "test".to_string(),
                created_at: "test".to_string(),
                updated_at: "test".to_string(),
            },
            EmailCategory {
                id: 2,
                name: "marketing_vendor".to_string(),
                description: "Marketing".to_string(),
                status: "active".to_string(),
                source: "test".to_string(),
                created_at: "test".to_string(),
                updated_at: "test".to_string(),
            },
        ],
        topics: vec![EmailTopic {
            id: 1,
            name: "general".to_string(),
            description: "General".to_string(),
            status: "active".to_string(),
            source: "test".to_string(),
            created_at: "test".to_string(),
            updated_at: "test".to_string(),
        }],
    }
}

fn resolved_classification(
    classification: &EmailClassification,
) -> ResolvedEmailClassification {
    let category = match classification.category.as_str() {
        "marketing_vendor" => ("marketing_vendor", 2),
        _ => ("question", 1),
    };
    ResolvedEmailClassification {
        category_id: category.1,
        category: category.0.to_string(),
        topic_ids: vec![1],
        topics: vec!["general".to_string()],
        reason: classification.reason.clone(),
        confidence: classification.confidence,
    }
}

fn email_rule(action: EmailRuleAction) -> EmailRule {
    EmailRule {
        id: 1,
        mailbox_id: "support".to_string(),
        name: "Test rule".to_string(),
        category_id: 1,
        category: "question".to_string(),
        topic_ids: vec![],
        topics: vec![],
        action,
        reply_goal: "Handle this with the configured rule".to_string(),
        enabled: true,
        priority: 10,
        created_at: "test".to_string(),
        updated_at: "test".to_string(),
    }
}

fn safe_scan() -> SafetyScanResult {
    SafetyScanResult {
        category: SafetyCategory::Safe,
        reason: "routine".to_string(),
        confidence: 0.9,
    }
}

fn reply_action() -> OutboundAction {
    OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Hello".to_string(),
        body: "Known answer".to_string(),
        reason: "memory supported answer".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    }
}

fn forward_action() -> OutboundAction {
    OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: vec!["human@example.com".to_string()],
        subject: "Fwd: cited agent memory".to_string(),
        body: "Josh asked whether a short meeting would be useful.".to_string(),
        reason: "requires Mark's judgment".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    }
}

fn noop_action() -> OutboundAction {
    OutboundAction {
        kind: OutboundActionKind::Noop,
        recipients: vec![],
        subject: String::new(),
        body: String::new(),
        reason: "no safe action".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    }
}

#[test]
fn builds_poll_plan_for_enabled_mailboxes() {
    let plans = poll_plans(&config());
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].mailbox_id, "support");
    assert_eq!(plans[0].interval, Duration::from_secs(30));
}

#[test]
fn next_poll_delay_uses_shortest_enabled_mailbox_interval() {
    assert_eq!(next_poll_delay(&config()), Duration::from_secs(30));
}

#[test]
fn next_poll_delay_defaults_when_no_mailboxes_are_enabled() {
    let mut config = config();
    config.mailboxes[0].enabled = false;
    assert_eq!(next_poll_delay(&config), Duration::from_secs(60));
}

#[test]
fn sender_precheck_blocks_banned_domains() {
    assert_eq!(
        precheck_sender("person@blocked.test", &config()),
        SenderPrecheck::Banned {
            reason: "sender is on the banned sender list".to_string()
        }
    );
}

#[test]
fn sender_precheck_allows_unlisted_senders() {
    assert_eq!(
        precheck_sender("person@example.com", &config()),
        SenderPrecheck::Allowed
    );
}

#[test]
fn forward_runtime_fields_strip_model_supplied_thread_headers() {
    let config = config();
    let message = inbound(
        65,
        "Josh <joshua.kappler@gmail.com>",
        "cited agent memory",
        "Original memo-engine meeting request.",
    );
    let mut action = forward_action();
    action.message_id = Some("<model-supplied@example.com>".to_string());
    action.in_reply_to = Some("<unreviewed-parent@example.com>".to_string());
    action.references = vec!["<unreviewed-root@example.com>".to_string()];

    let action = action_with_runtime_fields(&config.mailboxes[0], &message, &action);

    assert_eq!(action.kind, OutboundActionKind::Forward);
    assert_eq!(action.message_id, None);
    assert_eq!(action.in_reply_to, None);
    assert!(action.references.is_empty());
    assert!(action
        .body
        .contains("---------- Forwarded message ---------"));
    assert!(action
        .body
        .contains("Original memo-engine meeting request."));
}

#[tokio::test]
async fn run_once_logs_mailbox_count() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;
    let events = logger.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].action, "worker_poll_plan");
    assert_eq!(events[0].status, "mailboxes=1");
    assert_eq!(events[1].action, "imap_fetch");
    assert_eq!(events[1].status, "messages=0");
}

#[tokio::test]
async fn safe_message_replies_and_marks_seen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(42, "person@example.com", "Hello", "Question")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Reply);
    assert!(sent[0].body.contains(crate::mail::AUTOMATED_REPLY_NOTICE));
    assert_eq!(sent[0].in_reply_to, Some("<42@example.com>".to_string()));
    assert_eq!(sent[0].references, vec!["<42@example.com>".to_string()]);
    assert!(sent[0]
        .message_id
        .as_deref()
        .is_some_and(|message_id| message_id.ends_with("@example.com>")));
    assert_eq!(mail.seen()[0].uid, 42);
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 1,
            agent_decision: 1,
            rule_decision: 0,
            outbound_review: 0,
        }
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "smtp_send" && event.status == "replied"));
}

#[tokio::test]
async fn marketing_vendor_message_uses_seeded_auto_decline_rule() {
    let logger = crate::logging::MemoryLogger::default();
    let processing = MemoryProcessingStore::default();
    let message = inbound(
        66,
        "agency@example.com",
        "Grow your audience",
        "We can sell you a paid PR campaign.",
    );
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message]);
    let mut rule_action = reply_action();
    rule_action.body =
        "Thanks for reaching out, but I am not interested in paid marketing services."
            .to_string();
    rule_action.reason = "matched marketing vendor auto-decline rule".to_string();
    let mut decisions = fake_decisions(safe_scan(), forward_action());
    decisions.classification = marketing_classification();
    decisions.rule_decision = AgentDecision {
        action: rule_action,
        safety_notes: "rule-generated response".to_string(),
    };

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Reply);
    assert!(sent[0]
        .body
        .contains("not interested in paid marketing services"));
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 1,
            agent_decision: 0,
            rule_decision: 1,
            outbound_review: 0,
        }
    );
    assert_eq!(
        processing.email_classification(&key),
        Some(crate::storage::StoredEmailClassification {
            category: "marketing_vendor".to_string(),
            topics: vec!["general".to_string()],
            reason: "offers a paid promotion service".to_string(),
            confidence: 94,
            decision_source: "rule".to_string(),
            matched_rule_name: Some("Auto-decline marketing/vendor outreach".to_string()),
        })
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "rule_match" && event.status == "matched"));
}

#[tokio::test]
async fn human_review_request_skips_matching_rule() {
    let logger = crate::logging::MemoryLogger::default();
    let message = inbound(
        67,
        "agency@example.com",
        "Grow your audience",
        "We can sell you a paid PR campaign. Please escalate to human.",
    );
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message]);
    let mut decisions = fake_decisions(safe_scan(), reply_action());
    decisions.classification = marketing_classification();
    decisions.rule_decision = AgentDecision {
        action: reply_action(),
        safety_notes: "rule-generated response".to_string(),
    };
    let processing = MemoryProcessingStore::default();

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert_eq!(sent[0].recipients, vec!["human@example.com"]);
    assert_eq!(sent[0].reason, "sender requested human review");
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 1,
            agent_decision: 0,
            rule_decision: 0,
            outbound_review: 0,
        }
    );
    assert_eq!(
        processing.email_classification(&key),
        Some(crate::storage::StoredEmailClassification {
            category: "marketing_vendor".to_string(),
            topics: vec!["general".to_string()],
            reason: "offers a paid promotion service".to_string(),
            confidence: 94,
            decision_source: "human_review".to_string(),
            matched_rule_name: None,
        })
    );
    let resolved = processing
        .resolve_email_classification(&marketing_classification())
        .await
        .unwrap();
    assert!(processing
        .find_matching_email_rule("support", &resolved)
        .await
        .unwrap()
        .is_some());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "rule_match" && event.status == "skipped"));
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "human_review" && event.status == "forward"));
}

#[tokio::test]
async fn taxonomy_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(67, "person@example.com", "Hello", "Question")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_fail_taxonomy();

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 0,
            agent_decision: 0,
            rule_decision: 0,
            outbound_review: 0,
        }
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "email_taxonomy" && event.status == "failed"));
}

#[tokio::test]
async fn missing_classifier_prompt_uses_agent_without_taxonomy() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(68, "person@example.com", "Hello", "Question")]);
    let mut decisions = fake_decisions(safe_scan(), reply_action());
    decisions.missing_classifier_prompt = true;
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_fail_taxonomy();

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(mail.sent().len(), 1);
    assert_eq!(mail.seen()[0].uid, 68);
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 0,
            agent_decision: 1,
            rule_decision: 0,
            outbound_review: 0,
        }
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "email_classification" && event.status == "skipped"));
    assert!(!logger
        .events()
        .iter()
        .any(|event| event.action == "email_taxonomy" && event.status == "failed"));
}

#[tokio::test]
async fn missing_rule_prompt_records_agent_source_and_uses_agent() {
    let logger = crate::logging::MemoryLogger::default();
    let processing = MemoryProcessingStore::default();
    let message = inbound(
        69,
        "agency@example.com",
        "Grow your audience",
        "We can sell you a paid PR campaign.",
    );
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message]);
    let mut decisions = fake_decisions(safe_scan(), forward_action());
    decisions.classification = marketing_classification();
    decisions.missing_rule_prompt = true;

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 1,
            agent_decision: 1,
            rule_decision: 0,
            outbound_review: 0,
        }
    );
    assert_eq!(
        processing.email_classification(&key),
        Some(crate::storage::StoredEmailClassification {
            category: "marketing_vendor".to_string(),
            topics: vec!["general".to_string()],
            reason: "offers a paid promotion service".to_string(),
            confidence: 94,
            decision_source: "agent".to_string(),
            matched_rule_name: None,
        })
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "rule_match" && event.status == "matched"));
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "rule_decision" && event.status == "skipped"));
}

#[tokio::test]
async fn classification_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(70, "person@example.com", "Hello", "Question")]);
    let mut decisions = fake_decisions(safe_scan(), reply_action());
    decisions.fail_classification = true;
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]);

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            classify_email: 1,
            agent_decision: 0,
            rule_decision: 0,
            outbound_review: 0,
        }
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "email_classification" && event.status == "failed"));
}

#[tokio::test]
async fn classification_resolve_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(71, "person@example.com", "Hello", "Question")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_fail_resolve();

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert!(logger.events().iter().any(|event| {
        event.action == "email_classification_resolve" && event.status == "failed"
    }));
}

#[tokio::test]
async fn rule_match_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(72, "person@example.com", "Hello", "Question")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_fail_rule_match();

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "rule_match" && event.status == "failed"));
}
