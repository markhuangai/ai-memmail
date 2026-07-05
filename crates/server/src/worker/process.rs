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

    let scan = match run_ai_step(
        processing,
        &message,
        "safety_scan",
        decisions.safety_scan(config, mailbox, &message),
    )
    .await
    {
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

    let needs_human_review = human_review_requested(&message);
    let classification_context = match classify_message_for_rules(
        config,
        mailbox,
        logger,
        run_id,
        decisions,
        processing,
        &message,
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
                decisions.rule_decision(config, mailbox, &message, &context.raw, rule),
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
                        &message,
                        decision_started,
                    )
                    .await
                    {
                        Some(decision) => decision,
                        None => return,
                    }
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
                &message,
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
            &message,
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

async fn classify_message_for_rules(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    needs_human_review: bool,
) -> Option<Option<ClassificationContext>> {
    let started = Instant::now();
    match decisions.classifier_prompt_missing(config) {
        Ok(true) => {
            log_optional_prompt_skip(
                logger,
                run_id,
                message,
                "email_classification",
                started.elapsed(),
                format!(
                    "optional prompt missing: {}",
                    config.prompts.email_classifier.display()
                ),
            )
            .await;
            return Some(None);
        }
        Ok(false) => {}
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                message,
                "email_classification",
                started,
                error.to_string(),
            )
            .await;
            return None;
        }
    }

    let taxonomy = match processing.active_email_taxonomy().await {
        Ok(taxonomy) => taxonomy,
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                message,
                "email_taxonomy",
                started,
                error.to_string(),
            )
            .await;
            return None;
        }
    };
    let raw = match run_ai_step(
        processing,
        message,
        "email_classification",
        decisions.classify_email(config, mailbox, message, &taxonomy),
    )
    .await
    {
        Ok(classification) => classification,
        Err(error) if ai_error_is_missing_prompt(&error) => {
            log_optional_prompt_skip(
                logger,
                run_id,
                message,
                "email_classification",
                started.elapsed(),
                error.to_string(),
            )
            .await;
            return Some(None);
        }
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                message,
                "email_classification",
                started,
                error.to_string(),
            )
            .await;
            return None;
        }
    };
    let resolved = match processing.resolve_email_classification(&raw).await {
        Ok(classification) => classification,
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                message,
                "email_classification_resolve",
                started,
                error.to_string(),
            )
            .await;
            return None;
        }
    };
    log_email_classification(logger, run_id, message, &resolved, started.elapsed()).await;

    let mut matched_rule = if needs_human_review {
        None
    } else {
        match processing
            .find_matching_email_rule(&mailbox.id, &resolved)
            .await
        {
            Ok(rule) => rule,
            Err(error) => {
                log_retryable_message_error(
                    logger,
                    processing,
                    run_id,
                    message,
                    "rule_match",
                    started,
                    error.to_string(),
                )
                .await;
                return None;
            }
        }
    };
    let mut decision_source = if needs_human_review {
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
                message,
                "rule_match",
                "skipped",
                started.elapsed(),
                Some("sender requested human review".to_string()),
            ))
            .await;
    } else {
        log_rule_match(logger, run_id, message, matched_rule.as_ref(), started.elapsed()).await;
        if matched_rule.is_some() {
            match decisions.rule_prompt_missing(config) {
                Ok(true) => {
                    log_optional_prompt_skip(
                        logger,
                        run_id,
                        message,
                        "rule_decision",
                        started.elapsed(),
                        format!(
                            "optional prompt missing: {}",
                            config.prompts.rule_action.display()
                        ),
                    )
                    .await;
                    matched_rule = None;
                    decision_source = "agent";
                }
                Ok(false) => {}
                Err(error) => {
                    log_retryable_message_error(
                        logger,
                        processing,
                        run_id,
                        message,
                        "rule_decision",
                        started,
                        error.to_string(),
                    )
                    .await;
                    return None;
                }
            }
        }
    }
    record_email_classification_for_history(
        processing,
        logger,
        run_id,
        message,
        &resolved,
        decision_source,
        matched_rule.as_ref(),
    )
    .await;

    Some(Some(ClassificationContext {
        raw,
        resolved,
        matched_rule,
    }))
}

async fn agent_decision_or_retry(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    started: Instant,
) -> Option<AgentDecision> {
    match run_ai_step(
        processing,
        message,
        "agent_decision",
        decisions.agent_decision(config, mailbox, message),
    )
    .await
    {
        Ok(decision) => {
            log_agent_decision(logger, run_id, message, &decision, started.elapsed()).await;
            Some(decision)
        }
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                message,
                "agent_decision",
                started,
                error.to_string(),
            )
            .await;
            None
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
