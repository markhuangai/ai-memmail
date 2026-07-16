#[tokio::test]
async fn rule_decision_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(71, "person@example.com", "Hello", "Question")]);
    let mut decisions = fake_decisions(safe_scan(), reply_action());
    decisions.fail_rule = true;
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

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
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
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "rule_decision" && event.status == "failed"));
}

#[tokio::test]
async fn classification_history_persist_failure_is_logged_without_blocking_send() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(72, "person@example.com", "Hello", "Question")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed])
        .with_fail_classification_record();

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
    assert_eq!(mail.seen()[0].uid, 72);
    assert!(logger.events().iter().any(|event| {
        event.action == "email_classification_persist" && event.status == "failed"
    }));
}

#[tokio::test]
async fn agent_forward_includes_original_message_body_before_send() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(
        62,
        "Josh <joshua.kappler@gmail.com>",
        "cited agent memory",
        "Original memo-engine meeting request.",
    )]);
    let decisions = fake_decisions(safe_scan(), forward_action());

    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert_eq!(
        sent[0].body,
        "Josh asked whether a short meeting would be useful.\n\n---------- Forwarded message ---------\nFrom: Josh <joshua.kappler@gmail.com>\nSubject: cited agent memory\nMessage-ID: <62@example.com>\nUID: 1:62\n\nOriginal memo-engine meeting request."
    );
    assert_eq!(mail.seen()[0].uid, 62);
}

#[tokio::test]
async fn messages_in_one_poll_get_distinct_run_ids() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![
        inbound(60, "first@example.com", "First", "Question"),
        inbound(61, "second@example.com", "Second", "Question"),
    ]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed, FakeClaimOutcome::Claimed]);

    run_once_with_store(
        &config(),
        &logger,
        "poll-run",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    let run_ids = processing.run_ids();
    assert_eq!(run_ids.len(), 2);
    assert_ne!(run_ids[0], run_ids[1]);
    uuid::Uuid::parse_str(&run_ids[0]).unwrap();
    uuid::Uuid::parse_str(&run_ids[1]).unwrap();
}

#[tokio::test]
async fn enabled_outbound_review_approves_reply_before_send() {
    let mut config = config();
    config.ai.review.enabled = true;
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(56, "person@example.com", "Hello", "Question")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Reply);
    let reviewed = decisions.reviewed_actions();
    assert_eq!(reviewed.len(), 1);
    assert_eq!(reviewed[0].kind, OutboundActionKind::Reply);
    assert!(reviewed[0]
        .body
        .contains(crate::mail::AUTOMATED_REPLY_NOTICE));
    assert_eq!(
        reviewed[0].in_reply_to,
        Some("<56@example.com>".to_string())
    );
    assert_eq!(mail.sent()[0].body, reviewed[0].body);
    assert_eq!(mail.sent()[0].message_id, reviewed[0].message_id);
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "outbound_review" && event.status == "approved"));
}

#[tokio::test]
async fn escalation_phrase_routes_automated_reply_followup_to_human_forward() {
    let config = config();
    let logger = crate::logging::MemoryLogger::default();
    let mut message = inbound(64, "person@example.com", "Re: Hello", "escalation to human");
    message.metadata.in_reply_to = Some("<auto-reply@example.com>".to_string());
    message.metadata.references = vec!["<42@example.com>".to_string()];
    let action = crate::ai::forward_decision(
        &config.mailboxes[0],
        &message,
        "sender requested human review",
    )
    .action;
    let mail = FakeMail::new(vec![message]);
    let decisions = fake_decisions(safe_scan(), action);

    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert_eq!(sent[0].recipients, vec!["human@example.com"]);
    assert!(sent[0].body.contains("Human review requested"));
    assert!(sent[0].body.contains("escalation to human"));
}

#[tokio::test]
async fn outbound_review_receives_composed_forward_body() {
    let mut config = config();
    config.ai.review.enabled = true;
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(
        63,
        "Josh <joshua.kappler@gmail.com>",
        "cited agent memory",
        "Original memo-engine meeting request.",
    )]);
    let decisions = fake_decisions(safe_scan(), forward_action());

    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    let reviewed = decisions.reviewed_actions();
    assert_eq!(reviewed.len(), 1);
    assert_eq!(reviewed[0].kind, OutboundActionKind::Forward);
    assert_eq!(
        reviewed[0].body,
        "Josh asked whether a short meeting would be useful.\n\n---------- Forwarded message ---------\nFrom: Josh <joshua.kappler@gmail.com>\nSubject: cited agent memory\nMessage-ID: <63@example.com>\nUID: 1:63\n\nOriginal memo-engine meeting request."
    );
    assert_eq!(mail.sent()[0].body, reviewed[0].body);
}

#[tokio::test]
async fn enabled_outbound_review_rejection_forwards_to_human_reviewer() {
    let mut config = config();
    config.ai.review.enabled = true;
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(57, "person@example.com", "Hello", "Question")]);
    let decisions = rejecting_review_decisions();
    run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert_eq!(sent[0].recipients, vec!["human@example.com"]);
    assert!(sent[0].reason.contains("outbound review rejected"));
    assert!(!sent[0].body.contains("Known answer"));
    assert_eq!(mail.seen()[0].uid, 57);
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "outbound_review" && event.status == "rejected"));
}

#[tokio::test]
async fn enabled_outbound_review_failure_leaves_message_unseen() {
    let mut config = config();
    config.ai.review.enabled = true;
    let logger = crate::logging::MemoryLogger::default();
    let processing = MemoryProcessingStore::default();
    let message = inbound(58, "person@example.com", "Hello", "Question");
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message]);
    let decisions = failing_review_decisions();
    run_once_with_store(&config, &logger, "run-test", &mail, &decisions, &processing).await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert_eq!(
        processing.status(&key),
        Some(PROCESSING_STATUS_RETRYABLE_FAILED.to_string())
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "outbound_review" && event.status == "failed"));
}

#[tokio::test]
async fn unsafe_message_is_quarantined_and_forwarded() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(
        43,
        "person@example.com",
        "Policy override attempt",
        "Sensitive request",
    )]);
    let decisions = fake_decisions(
        SafetyScanResult {
            category: SafetyCategory::PromptInjection,
            reason: "tries to override policy".to_string(),
            confidence: 0.98,
        },
        reply_action(),
    );
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert!(sent[0].subject.starts_with("[Potential jailbreak]"));
    assert_eq!(mail.seen()[0].uid, 43);
}

#[tokio::test]
async fn unsafe_message_persists_safety_and_sender_review_state() {
    let logger = crate::logging::MemoryLogger::default();
    let processing = MemoryProcessingStore::default();
    let message = inbound(
        59,
        "person@example.com",
        "Policy override attempt",
        "Sensitive request",
    );
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message.clone()]);
    let decisions = fake_decisions(
        SafetyScanResult {
            category: SafetyCategory::PromptInjection,
            reason: "tries to override policy".to_string(),
            confidence: 0.98,
        },
        reply_action(),
    );
    run_once_with_store(
        &config(),
        &logger,
        "run-test",
        &mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(
        processing.safety_result(&key),
        Some(crate::storage::StoredSafetyResult {
            category: "prompt_injection".to_string(),
            reason: "tries to override policy".to_string()
        })
    );
    assert_eq!(
        processing.sender_review("person@example.com"),
        Some(crate::storage::SenderReviewRecord {
            mailbox_id: "support".to_string(),
            reason: "tries to override policy".to_string()
        })
    );
    assert_eq!(mail.sent().len(), 1);
    assert_eq!(mail.seen()[0].uid, 59);
}

#[tokio::test]
async fn banned_sender_is_forwarded_before_ai_processing() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(
        44,
        "person@blocked.test",
        "Routine",
        "Please answer",
    )]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, OutboundActionKind::Forward);
    assert!(sent[0].body.contains("sender is on the banned sender list"));
    assert_eq!(mail.seen()[0].uid, 44);
}

#[tokio::test]
async fn fetch_failure_is_logged() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![]).with_fail_fetch();
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "imap_fetch" && event.status == "failed"));
}

#[tokio::test]
async fn safety_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let processing = MemoryProcessingStore::default();
    let message = inbound(45, "person@example.com", "Question", "Body");
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message]);
    let decisions = failing_safety_decisions();
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
        processing.status(&key),
        Some(PROCESSING_STATUS_RETRYABLE_FAILED.to_string())
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "safety_scan" && event.status == "failed"));
}

#[tokio::test]
async fn agent_failure_leaves_message_unseen() {
    let logger = crate::logging::MemoryLogger::default();
    let processing = MemoryProcessingStore::default();
    let message = inbound(46, "person@example.com", "Question", "Body");
    let key = message.metadata.dedupe_key();
    let mail = FakeMail::new(vec![message]);
    let decisions = failing_agent_decisions();
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
        processing.status(&key),
        Some(PROCESSING_STATUS_RETRYABLE_FAILED.to_string())
    );
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "agent_decision" && event.status == "failed"));
}

#[tokio::test]
async fn agent_timeout_heartbeats_and_marks_retryable_failed() {
    let logger = crate::logging::MemoryLogger::default();
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]);
    let message = inbound(146, "person@example.com", "Question", "Body");
    let mail = FakeMail::new(vec![message]);
    let decisions = fake_decisions(safe_scan(), reply_action()).with_hang_agent();
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
    assert!(processing.touch_count() > 0);
    assert!(processing
        .statuses()
        .contains(&PROCESSING_STATUS_RETRYABLE_FAILED.to_string()));
    assert!(logger.events().iter().any(|event| {
        event.action == "agent_decision"
            && event.status == "failed"
            && event
                .detail
                .as_ref()
                .is_some_and(|detail| detail.contains("timed out"))
    }));
}

#[tokio::test]
async fn send_failure_does_not_mark_seen() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(47, "person@example.com", "Question", "Body")])
        .with_fail_send();
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    assert!(mail.sent().is_empty());
    assert!(mail.seen().is_empty());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "smtp_send" && event.status == "failed"));
}

#[tokio::test]
async fn mark_seen_failure_is_logged_after_send() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(48, "person@example.com", "Question", "Body")])
        .with_fail_mark_seen();
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

    assert_eq!(mail.sent().len(), 1);
    assert!(mail.seen().is_empty());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "imap_mark_seen" && event.status == "failed"));
}

#[tokio::test]
async fn sent_message_is_not_sent_again_when_mark_seen_failed() {
    let message = inbound(50, "person@example.com", "Question", "Body");
    let processing = MemoryProcessingStore::default();
    let first_logger = crate::logging::MemoryLogger::default();
    let first_mail = FakeMail::new(vec![message.clone()]).with_fail_mark_seen();
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with_store(
        &config(),
        &first_logger,
        "run-test",
        &first_mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(first_mail.sent().len(), 1);
    assert!(first_mail.seen().is_empty());
    assert_eq!(
        processing.status(&message.metadata.dedupe_key()),
        Some("replied".to_string())
    );

    let second_logger = crate::logging::MemoryLogger::default();
    let second_mail = FakeMail::new(vec![message]);
    run_once_with_store(
        &config(),
        &second_logger,
        "run-test-2",
        &second_mail,
        &decisions,
        &processing,
    )
    .await;

    assert!(second_mail.sent().is_empty());
    assert_eq!(second_mail.seen()[0].uid, 50);
    assert!(second_logger
        .events()
        .iter()
        .any(|event| event.action == "processing_claim" && event.status == "dedupe_skip"));
}

#[tokio::test]
async fn send_failure_can_be_retried_by_later_poll() {
    let message = inbound(51, "person@example.com", "Question", "Body");
    let processing = MemoryProcessingStore::default();
    let first_logger = crate::logging::MemoryLogger::default();
    let first_mail = FakeMail::new(vec![message.clone()]).with_fail_send();
    let decisions = fake_decisions(safe_scan(), reply_action());
    run_once_with_store(
        &config(),
        &first_logger,
        "run-test",
        &first_mail,
        &decisions,
        &processing,
    )
    .await;

    assert!(first_mail.sent().is_empty());
    assert_eq!(
        processing.status(&message.metadata.dedupe_key()),
        Some(PROCESSING_STATUS_SEND_FAILED.to_string())
    );

    let second_logger = crate::logging::MemoryLogger::default();
    let second_mail = FakeMail::new(vec![message]);
    run_once_with_store(
        &config(),
        &second_logger,
        "run-test-2",
        &second_mail,
        &decisions,
        &processing,
    )
    .await;

    assert_eq!(second_mail.sent().len(), 1);
    assert_eq!(second_mail.seen()[0].uid, 51);
}

#[tokio::test]
async fn in_progress_claim_defers_message_without_side_effects() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(52, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::InProgress]);
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
    assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "processing_claim" && event.status == "in_progress"));
}

#[tokio::test]
async fn claim_failure_logs_and_defers_message_without_side_effects() {
    let logger = crate::logging::MemoryLogger::default();
    let mail = FakeMail::new(vec![inbound(53, "person@example.com", "Question", "Body")]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Fail]);
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
    assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
    assert!(logger
        .events()
        .iter()
        .any(|event| event.action == "processing_claim" && event.status == "failed"));
}

#[tokio::test]
async fn active_thread_handoff_forwards_chain_without_classification_or_agent_decision() {
    let logger = crate::logging::MemoryLogger::default();
    let message = inbound(54, "Person <person@example.com>", "Project follow up", "Needs help");
    let thread_id = message.metadata.thread_id();
    let mail = FakeMail::new(vec![message.clone()]);
    let decisions = fake_decisions(safe_scan(), reply_action());
    let processing =
        FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_handoff(
            crate::storage::ThreadHandoff {
                mailbox_id: "support".to_string(),
                thread_id: thread_id.clone(),
                destination: "mark.personal@example.com".to_string(),
                remote_target: "person@example.com".to_string(),
                state: "active".to_string(),
                last_error: None,
                updated_at: "memory".to_string(),
            },
        );

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
    assert_eq!(sent[0].recipients, vec!["mark.personal@example.com"]);
    assert_eq!(sent[0].reply_to.as_deref(), Some("person@example.com"));
    assert_eq!(sent[0].in_reply_to.as_deref(), Some("<54@example.com>"));
    assert!(sent[0].body.contains("---------- Conversation handoff ---------"));
    assert!(sent[0].body.contains("Needs help"));
    assert_eq!(processing.statuses(), vec![PROCESSING_STATUS_HANDED_OFF]);
    assert_eq!(mail.seen()[0].uid, 54);
    assert_eq!(
        decisions.call_counts(),
        DecisionCallCounts {
            safety_scan: 1,
            ..DecisionCallCounts::default()
        }
    );
    assert_eq!(processing.handoff_deliveries().len(), 1);
    assert!(!processing.handoff_deliveries()[0]
        .outbound_message_id
        .is_empty());
}
