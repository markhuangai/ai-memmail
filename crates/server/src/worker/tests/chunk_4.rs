#[tokio::test]
async fn accepted_conditions_filter_messages_before_processing() {
    let mut config = config();
    config.mailboxes[0].accepted_conditions = vec![AcceptedCondition {
        recipients: vec!["support@example.com".to_string()],
        subject_regex: vec!["(?i)billing".to_string()],
    }];
    let mut matching = inbound(
        80,
        "person@example.com",
        "Billing question",
        "Can you help with billing?",
    );
    matching.metadata.recipients = vec!["support@example.com".to_string()];
    let mut wrong_recipient = inbound(
        81,
        "person@example.com",
        "Billing question",
        "Can you help with billing?",
    );
    wrong_recipient.metadata.recipients = vec!["other@example.com".to_string()];
    let mut wrong_subject = inbound(
        82,
        "person@example.com",
        "Sales question",
        "Can you help with sales?",
    );
    wrong_subject.metadata.recipients = vec!["support@example.com".to_string()];
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![wrong_recipient, matching, wrong_subject]);
    let decisions = fake_decisions(safe_scan(), reply_action());

    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    assert_eq!(mail.sent().len(), 1);
    assert_eq!(
        mail.seen().iter().map(|key| key.uid).collect::<Vec<_>>(),
        vec![80]
    );
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
    assert!(logger.events().iter().any(|event| {
        event.action == "imap_fetch"
            && event.status == "messages=1"
            && event.detail.as_deref() == Some("filtered_by_accepted_conditions=2")
    }));
}

#[tokio::test]
async fn already_finished_claim_marks_seen_without_sending() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(54, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::AlreadyFinished]);
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
    assert_eq!(mail.seen()[0].uid, 54);
    assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "processing_claim" && event.status == "dedupe_skip"));
}

#[tokio::test]
async fn processing_update_failure_is_logged_after_successful_send() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(55, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_fail_update();
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
    assert_eq!(mail.seen()[0].uid, 55);
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "processing_update" && event.status == "failed"));
}

#[tokio::test]
async fn noop_marks_seen_without_sending() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(49, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), noop_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    assert!(mail.sent().is_empty());
    assert_eq!(mail.seen()[0].uid, 49);
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "imap_mark_seen" && event.status == "noop"));
}

#[test]
fn load_and_plan_reads_config_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    config().save(&path).unwrap();
    let plans = load_and_plan(&path).unwrap();
    assert_eq!(plans[0].mailbox_id, "support");
}

#[test]
fn safety_disposition_controls_human_review_forward() {
    let decision = crate::safety::decide(&SafetyScanResult {
        category: SafetyCategory::Jailbreak,
        reason: "tries to override policy".to_string(),
        confidence: 0.95,
    });
    assert!(should_forward_for_human_review(&decision));
}

#[tokio::test]
async fn sent_sync_must_complete_before_inbox_processing() {
    let mut config = config();
    config.mailboxes[0].imap.sent_backfill_days = 30;
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(90, "person@example.com", "Question", "Body")])
        .with_sent_batch(SentFetchBatch {
            folder_name: "Sent".to_string(),
            uid_validity: 7,
            messages: vec![],
            complete: true,
        });
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]);

    run_once_with_store(
        &config,
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(mail.sent().len(), 1);
    let sent_fetches = mail.sent_fetches();
    assert_eq!(sent_fetches.len(), 1);
    assert_eq!(sent_fetches[0].0, None);
    assert_eq!(sent_fetches[0].2, 200);
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "imap_sent_sync" && event.status == "complete"));
}

#[tokio::test]
async fn sent_sync_failure_fails_closed() {
    let mut config = config();
    config.mailboxes[0].imap.sent_backfill_days = 30;
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(91, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action());

    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "imap_sent_sync" && event.status == "failed_closed"));
}

#[tokio::test]
async fn oversized_current_message_forwards_without_ai() {
    let logger = crate::logging::MemoryLogger::default();
    let message = inbound(
        92,
        "person@example.com",
        "Large reply",
        &"x".repeat(crate::ai::MAX_SERIALIZED_PROMPT_CHARS + 1),
    );
    let mail = FakeMail::new(vec![message]);
    let decisions = fake_decisions(safe_scan(), reply_action());

    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    assert_eq!(mail.sent()[0].kind, OutboundActionKind::Forward);
    assert_eq!(mail.sent()[0].recipients, vec!["human@example.com"]);
    assert_eq!(mail.seen()[0].uid, 92);
    assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
}

#[tokio::test]
async fn truncated_stored_history_forwards_before_decision_ai() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(93, "person@example.com", "Re: Question", "Answer")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed])
        .with_thread_context(ThreadContext {
            thread_id: "<root@example.com>".to_string(),
            messages: vec![ThreadMessage {
                direction: MessageDirection::Outbound,
                message_id: Some("<root@example.com>".to_string()),
                in_reply_to: None,
                references: vec![],
                from_addr: "support@example.com".to_string(),
                recipients: vec!["person@example.com".to_string()],
                subject: "Question".to_string(),
                authored_text: "stored prefix".to_string(),
                body_truncated: true,
                timestamp: 1,
            }],
        });

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(mail.sent()[0].kind, OutboundActionKind::Forward);
    assert_eq!(mail.seen()[0].uid, 93);
    assert_eq!(decisions.call_counts().safety_scan, 1);
    assert_eq!(decisions.call_counts().classify_email, 0);
    assert_eq!(decisions.call_counts().agent_decision, 0);
}

#[tokio::test]
async fn provider_context_limit_forwards_instead_of_retrying() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(94, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action())
        .with_context_limit_at("agent_decision");
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

    assert_eq!(mail.sent()[0].kind, OutboundActionKind::Forward);
    assert_eq!(mail.seen()[0].uid, 94);
    assert_eq!(processing.statuses(), vec!["forwarded"]);
}

#[tokio::test]
async fn classification_context_limit_forwards_instead_of_retrying() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(96, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action())
        .with_context_limit_at("email_classification");
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

    assert_eq!(mail.sent()[0].kind, OutboundActionKind::Forward);
    assert_eq!(mail.seen()[0].uid, 96);
    assert_eq!(decisions.call_counts().safety_scan, 1);
    assert_eq!(decisions.call_counts().classify_email, 1);
    assert_eq!(decisions.call_counts().agent_decision, 0);
    assert_eq!(processing.statuses(), vec!["forwarded"]);
}

#[tokio::test]
async fn rule_context_limit_forwards_instead_of_retrying() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(97, "person@example.com", "Question", "Body")]);
    let decisions =
        fake_decisions(safe_scan(), reply_action()).with_context_limit_at("rule_decision");
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed])
        .with_matched_rule(email_rule(EmailRuleAction::Reply));

    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(mail.sent()[0].kind, OutboundActionKind::Forward);
    assert_eq!(mail.seen()[0].uid, 97);
    assert_eq!(decisions.call_counts().rule_decision, 1);
    assert_eq!(decisions.call_counts().agent_decision, 0);
    assert_eq!(processing.statuses(), vec!["forwarded"]);
}

#[tokio::test]
async fn outbound_review_context_limit_replaces_reply_with_human_forward() {
    let mut config = config();
    config.ai.review.enabled = true;
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(95, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action())
        .with_context_limit_at("outbound_review");

    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    assert_eq!(mail.sent()[0].kind, OutboundActionKind::Forward);
    assert_eq!(mail.sent()[0].recipients, vec!["human@example.com"]);
    assert_eq!(mail.seen()[0].uid, 95);
    assert!(logger.events().iter().any(|event| {
        event.action == "outbound_review" && event.status == "context_limit_forward"
    }));
}
