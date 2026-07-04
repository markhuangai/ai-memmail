async fn process_mailbox(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
) {
    let started = Instant::now();
    match mail.fetch_unseen(mailbox, 10).await {
        Ok(messages) => {
            logger
                .log(mailbox_event(
                    LogLevel::Info,
                    run_id,
                    &mailbox.id,
                    "imap_fetch",
                    format!("messages={}", messages.len()),
                    started.elapsed(),
                    None,
                ))
                .await;
            for message in messages {
                let message_run_id = Uuid::new_v4().to_string();
                process_message(
                    config,
                    mailbox,
                    logger,
                    &message_run_id,
                    mail,
                    decisions,
                    processing,
                    message,
                )
                .await;
            }
        }
        Err(error) => {
            logger
                .log(mailbox_event(
                    LogLevel::Error,
                    run_id,
                    &mailbox.id,
                    "imap_fetch",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
        }
    }
}

async fn process_message(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: InboundMessage,
) {
    let started = Instant::now();
    match precheck_sender(&message.metadata.from_addr, config) {
        SenderPrecheck::Banned { reason } => {
            let action = safety_forward_action(mailbox, &message, &reason);
            if !claim_before_side_effect(processing, logger, run_id, mail, mailbox, &message).await
            {
                return;
            }
            send_and_mark_seen(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                &message,
                action,
                "banned_sender",
            )
            .await;
            return;
        }
        SenderPrecheck::Allowed => {}
    }

    if !claim_before_side_effect(processing, logger, run_id, mail, mailbox, &message).await {
        return;
    }

    let scan = match decisions.safety_scan(config, mailbox, &message).await {
        Ok(scan) => scan,
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    &message,
                    "safety_scan",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
    };
    log_safety_scan_result(logger, run_id, &message, &scan, started.elapsed()).await;
    record_safety_result_for_history(processing, logger, run_id, &message, &scan).await;
    let safety_decision = decide(&scan);
    if should_forward_for_human_review(&safety_decision) {
        let action = safety_forward_action(mailbox, &message, &safety_decision.reason);
        if !record_quarantine_state(
            processing,
            logger,
            run_id,
            &message,
            &scan,
            &safety_decision,
        )
        .await
        {
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
        send_and_mark_seen(
            mailbox,
            logger,
            run_id,
            mail,
            processing,
            &message,
            action,
            "quarantined",
        )
        .await;
        return;
    }

    let classification_started = Instant::now();
    let taxonomy = match processing.active_email_taxonomy().await {
        Ok(taxonomy) => taxonomy,
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    &message,
                    "email_taxonomy",
                    "failed",
                    classification_started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
    };
    let raw_classification = match decisions
        .classify_email(config, mailbox, &message, &taxonomy)
        .await
    {
        Ok(classification) => classification,
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    &message,
                    "email_classification",
                    "failed",
                    classification_started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
    };
    let classification = match processing
        .resolve_email_classification(&raw_classification)
        .await
    {
        Ok(classification) => classification,
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    &message,
                    "email_classification_resolve",
                    "failed",
                    classification_started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
    };
    log_email_classification(
        logger,
        run_id,
        &message,
        &classification,
        classification_started.elapsed(),
    )
    .await;
    let needs_human_review = human_review_requested(&message);
    let matched_rule = if needs_human_review {
        None
    } else {
        match processing
            .find_matching_email_rule(&mailbox.id, &classification)
            .await
        {
            Ok(rule) => rule,
            Err(error) => {
                logger
                    .log(message_event(
                        LogLevel::Error,
                        run_id,
                        &message,
                        "rule_match",
                        "failed",
                        classification_started.elapsed(),
                        Some(error.to_string()),
                    ))
                    .await;
                update_processing_status(
                    processing,
                    logger,
                    run_id,
                    &message,
                    PROCESSING_STATUS_RETRYABLE_FAILED,
                    None,
                )
                .await;
                return;
            }
        }
    };
    let decision_source = if needs_human_review {
        "human_review"
    } else if matched_rule.is_some() {
        "rule"
    } else {
        "agent"
    };
    if needs_human_review {
        logger
            .log(message_event(
                LogLevel::Info,
                run_id,
                &message,
                "rule_match",
                "skipped",
                classification_started.elapsed(),
                Some("sender requested human review".to_string()),
            ))
            .await;
    } else {
        log_rule_match(
            logger,
            run_id,
            &message,
            matched_rule.as_ref(),
            classification_started.elapsed(),
        )
        .await;
    }
    record_email_classification_for_history(
        processing,
        logger,
        run_id,
        &message,
        &classification,
        decision_source,
        matched_rule.as_ref(),
    )
    .await;

    let decision_started = Instant::now();
    let decision = if needs_human_review {
        let decision = forward_decision(mailbox, &message, "sender requested human review");
        log_decision(
            logger,
            run_id,
            &message,
            &decision,
            "human_review",
            decision_started.elapsed(),
        )
        .await;
        decision
    } else if let Some(rule) = &matched_rule {
        match decisions
            .rule_decision(config, mailbox, &message, &raw_classification, rule)
            .await
        {
            Ok(decision) => {
                log_decision(
                    logger,
                    run_id,
                    &message,
                    &decision,
                    "rule_decision",
                    decision_started.elapsed(),
                )
                .await;
                decision
            }
            Err(error) => {
                logger
                    .log(message_event(
                        LogLevel::Error,
                        run_id,
                        &message,
                        "rule_decision",
                        "failed",
                        decision_started.elapsed(),
                        Some(error.to_string()),
                    ))
                    .await;
                update_processing_status(
                    processing,
                    logger,
                    run_id,
                    &message,
                    PROCESSING_STATUS_RETRYABLE_FAILED,
                    None,
                )
                .await;
                return;
            }
        }
    } else {
        match decisions.agent_decision(config, mailbox, &message).await {
            Ok(decision) => {
                log_agent_decision(
                    logger,
                    run_id,
                    &message,
                    &decision,
                    decision_started.elapsed(),
                )
                .await;
                decision
            }
            Err(error) => {
                logger
                    .log(message_event(
                        LogLevel::Error,
                        run_id,
                        &message,
                        "agent_decision",
                        "failed",
                        decision_started.elapsed(),
                        Some(error.to_string()),
                    ))
                    .await;
                update_processing_status(
                    processing,
                    logger,
                    run_id,
                    &message,
                    PROCESSING_STATUS_RETRYABLE_FAILED,
                    None,
                )
                .await;
                return;
            }
        }
    };
    record_agent_decision_for_history(processing, logger, run_id, &message, &decision).await;
    let outbound_decision = decision_with_runtime_fields(mailbox, &message, &decision);

    match &outbound_decision.action.kind {
        OutboundActionKind::Noop => {
            record_outbound_action_for_history(
                processing,
                logger,
                run_id,
                &message,
                &outbound_decision.action,
            )
            .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                "noop",
                Some(&OutboundActionKind::Noop),
            )
            .await;
            mark_seen(mailbox, logger, run_id, mail, &message, "noop").await;
        }
        OutboundActionKind::Reply | OutboundActionKind::Forward => {
            let action = match reviewed_outbound_action(
                config,
                mailbox,
                logger,
                run_id,
                decisions,
                processing,
                &message,
                &outbound_decision,
            )
            .await
            {
                Some(action) => action,
                None => {
                    update_processing_status(
                        processing,
                        logger,
                        run_id,
                        &message,
                        PROCESSING_STATUS_RETRYABLE_FAILED,
                        None,
                    )
                    .await;
                    return;
                }
            };
            let status = outbound_status(&action.kind);
            send_and_mark_seen(
                mailbox, logger, run_id, mail, processing, &message, action, status,
            )
            .await;
        }
    }
}
