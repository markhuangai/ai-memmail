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
