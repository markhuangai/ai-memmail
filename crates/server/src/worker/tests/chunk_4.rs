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
