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
            let fetched_count = messages.len();
            let messages = messages
                .into_iter()
                .filter(|message| {
                    message_matches_accepted_conditions(message, &mailbox.accepted_conditions)
                })
                .collect::<Vec<_>>();
            let filtered_count = fetched_count.saturating_sub(messages.len());
            logger
                .log(mailbox_event(
                    LogLevel::Info,
                    run_id,
                    &mailbox.id,
                    "imap_fetch",
                    format!("messages={}", messages.len()),
                    started.elapsed(),
                    (filtered_count > 0)
                        .then(|| format!("filtered_by_accepted_conditions={filtered_count}")),
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

struct ClassificationContext {
    raw: EmailClassification,
    resolved: ResolvedEmailClassification,
    matched_rule: Option<EmailRule>,
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

    if message.plain_text.chars().count() > crate::ai::MAX_SERIALIZED_PROMPT_CHARS {
        forward_context_limit_message(
            mailbox,
            logger,
            run_id,
            mail,
            processing,
            &message,
            "current message exceeded the configured context limit",
        )
        .await;
        return;
    }

    let scan = match run_ai_step(
        processing,
        &message,
        "safety_scan",
        decisions.safety_scan(config, mailbox, &message),
    )
    .await
    {
        Ok(scan) => scan,
        Err(error) if ai_error_is_context_limit(&error) => {
            forward_context_limit_message(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                &message,
                "AI input exceeded the configured context limit",
            )
            .await;
            return;
        }
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

    let thread_context = match load_thread_context_or_retry(
        mailbox, logger, run_id, processing, &message,
    )
    .await
    {
        Some(context) => context,
        None => return,
    };
    if thread_context.has_truncated_body() {
        forward_context_limit_message(
            mailbox,
            logger,
            run_id,
            mail,
            processing,
            &message,
            "stored thread history was truncated",
        )
        .await;
        return;
    }
    match processing
        .active_thread_handoff(&mailbox.id, &thread_context.thread_id)
        .await
    {
        Ok(Some(handoff)) => {
            route_active_thread_handoff(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                &message,
                &thread_context,
                &handoff,
            )
            .await;
            return;
        }
        Ok(None) => {}
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                &message,
                "thread_handoff_lookup",
                started,
                error.to_string(),
            )
            .await;
            return;
        }
    }
    let needs_human_review = human_review_requested(&message);
    let classification_context = match classify_message_for_rules(
        config,
        mailbox,
        logger,
        run_id,
        mail,
        decisions,
        processing,
        &message,
        &thread_context,
        needs_human_review,
    )
    .await
    {
        Some(context) => context,
        None => return,
    };

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
    } else if let Some(context) = &classification_context {
        if let Some(rule) = &context.matched_rule {
            match run_ai_step(
                processing,
                &message,
                "rule_decision",
                decisions.rule_decision(
                    config,
                    mailbox,
                    &message,
                    &thread_context,
                    &context.raw,
                    rule,
                ),
            )
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
                Err(error) if ai_error_is_missing_prompt(&error) => {
                    log_optional_prompt_skip(
                        logger,
                        run_id,
                        &message,
                        "rule_decision",
                        decision_started.elapsed(),
                        error.to_string(),
                    )
                    .await;
                    record_email_classification_for_history(
                        processing,
                        logger,
                        run_id,
                        &message,
                        &context.resolved,
                        "agent",
                        None,
                    )
                    .await;
                    match agent_decision_or_retry(
                        config,
                        mailbox,
                        logger,
                        run_id,
                        decisions,
                        processing,
                        mail,
                        &message,
                        &thread_context,
                        decision_started,
                    )
                    .await
                    {
                        Some(decision) => decision,
                        None => return,
                    }
                }
                Err(error) if ai_error_is_context_limit(&error) => {
                    forward_context_limit_message(
                        mailbox,
                        logger,
                        run_id,
                        mail,
                        processing,
                        &message,
                        "AI input exceeded the configured context limit",
                    )
                    .await;
                    return;
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
            match agent_decision_or_retry(
                config,
                mailbox,
                logger,
                run_id,
                decisions,
                processing,
                mail,
                &message,
                &thread_context,
                decision_started,
            )
            .await
            {
                Some(decision) => decision,
                None => return,
            }
        }
    } else {
        match agent_decision_or_retry(
            config,
            mailbox,
            logger,
            run_id,
            decisions,
            processing,
            mail,
            &message,
            &thread_context,
            decision_started,
        )
        .await
        {
            Some(decision) => decision,
            None => return,
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
                &thread_context,
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

async fn log_optional_prompt_skip(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    action: &'static str,
    duration: Duration,
    detail: String,
) {
    logger
        .log(message_event(
            LogLevel::Warn,
            run_id,
            message,
            action,
            "skipped",
            duration,
            Some(detail),
        ))
        .await;
}

async fn log_retryable_message_error(
    logger: &dyn ActionLogger,
    processing: &dyn ProcessingStore,
    run_id: &str,
    message: &InboundMessage,
    action: &'static str,
    started: Instant,
    detail: String,
) {
    logger
        .log(message_event(
            LogLevel::Error,
            run_id,
            message,
            action,
            "failed",
            started.elapsed(),
            Some(detail),
        ))
        .await;
    update_processing_status(
        processing,
        logger,
        run_id,
        message,
        PROCESSING_STATUS_RETRYABLE_FAILED,
        None,
    )
    .await;
}

fn ai_error_is_missing_prompt(error: &AiError) -> bool {
    matches!(error, AiError::Prompt(prompt) if prompt.is_not_found())
}

fn ai_error_is_context_limit(error: &AiError) -> bool {
    matches!(error, AiError::ContextLengthExceeded(_))
}
