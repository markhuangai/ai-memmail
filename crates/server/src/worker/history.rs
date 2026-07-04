async fn log_safety_scan_result(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    scan: &SafetyScanResult,
    duration: Duration,
) {
    logger
        .log(message_event(
            LogLevel::Info,
            run_id,
            message,
            "safety_scan",
            crate::storage::safety_category_value(&scan.category),
            duration,
            Some(scan.reason.clone()),
        ))
        .await;
}

async fn log_agent_decision(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    decision: &AgentDecision,
    duration: Duration,
) {
    log_decision(
        logger,
        run_id,
        message,
        decision,
        "agent_decision",
        duration,
    )
    .await;
}

async fn log_decision(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    decision: &AgentDecision,
    action_name: &str,
    duration: Duration,
) {
    logger
        .log(message_event(
            LogLevel::Info,
            run_id,
            message,
            action_name,
            crate::storage::outbound_action_value(&decision.action.kind),
            duration,
            Some(decision.action.reason.clone()),
        ))
        .await;
}

async fn log_email_classification(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    classification: &ResolvedEmailClassification,
    duration: Duration,
) {
    logger
        .log(message_event(
            LogLevel::Info,
            run_id,
            message,
            "email_classification",
            &classification.category,
            duration,
            Some(format!(
                "topics={}; confidence={}; {}",
                classification.topics.join(","),
                classification.confidence,
                classification.reason
            )),
        ))
        .await;
}

async fn log_rule_match(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    rule: Option<&EmailRule>,
    duration: Duration,
) {
    let (status, detail) = match rule {
        Some(rule) => (
            "matched",
            Some(format!("{}: {}", rule.name, rule.reply_goal)),
        ),
        None => ("no_match", None),
    };
    logger
        .log(message_event(
            LogLevel::Info,
            run_id,
            message,
            "rule_match",
            status,
            duration,
            detail,
        ))
        .await;
}

async fn record_safety_result_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    scan: &SafetyScanResult,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_safety_result(&message.metadata.dedupe_key(), &scan.category, &scan.reason)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "safety_result_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_agent_decision_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    decision: &AgentDecision,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_agent_decision(&message.metadata.dedupe_key(), decision)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "agent_decision_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_email_classification_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    classification: &ResolvedEmailClassification,
    decision_source: &str,
    matched_rule: Option<&EmailRule>,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_email_classification(
            &message.metadata.dedupe_key(),
            classification,
            decision_source,
            matched_rule,
        )
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "email_classification_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_outbound_action_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    action: &OutboundAction,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_outbound_action(&message.metadata.dedupe_key(), action)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "outbound_action_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_outbound_review_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    status: &str,
    reason: &str,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_outbound_review(&message.metadata.dedupe_key(), status, reason)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "outbound_review_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_quarantine_state(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    scan: &SafetyScanResult,
    decision: &SafetyDecision,
) -> bool {
    let started = Instant::now();
    let mut persisted = true;
    if let Err(error) = processing
        .record_safety_result(&message.metadata.dedupe_key(), &scan.category, &scan.reason)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "safety_result_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
        persisted = false;
    }

    if decision.add_sender_to_review {
        let started = Instant::now();
        if let Err(error) = processing
            .upsert_sender_review(
                &message.metadata.from_addr,
                &message.metadata.mailbox_id,
                &decision.reason,
            )
            .await
        {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "sender_review_persist",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            persisted = false;
        }
    }

    persisted
}
