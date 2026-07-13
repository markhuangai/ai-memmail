async fn classify_message_for_rules(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    thread_context: &ThreadContext,
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
        Err(error) if ai_error_is_context_limit(&error) => {
            forward_context_limit_message(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                message,
                "AI input exceeded the configured context limit",
            )
            .await;
            return None;
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
        decisions.classify_email(config, mailbox, message, thread_context, &taxonomy),
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
        Err(error) if ai_error_is_context_limit(&error) => {
            forward_context_limit_message(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                message,
                "AI input exceeded the configured context limit",
            )
            .await;
            return None;
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
        log_rule_match(
            logger,
            run_id,
            message,
            matched_rule.as_ref(),
            started.elapsed(),
        )
        .await;
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
