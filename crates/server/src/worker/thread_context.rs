async fn load_thread_context_or_retry(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
) -> Option<ThreadContext> {
    let started = Instant::now();
    match processing.load_thread_context(mailbox, message).await {
        Ok(context) => Some(context),
        Err(error) => {
            log_retryable_message_error(
                logger,
                processing,
                run_id,
                message,
                "thread_context",
                started,
                error.to_string(),
            )
            .await;
            None
        }
    }
}

async fn agent_decision_or_retry(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    mail: &dyn MailTransport,
    message: &InboundMessage,
    thread_context: &ThreadContext,
    started: Instant,
) -> Option<AgentDecision> {
    match run_ai_step(
        processing,
        message,
        "agent_decision",
        decisions.agent_decision(config, mailbox, message, thread_context),
    )
    .await
    {
        Ok(decision) => {
            log_agent_decision(logger, run_id, message, &decision, started.elapsed()).await;
            Some(decision)
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
            None
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

async fn forward_context_limit_message(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    reason: &str,
) {
    let started = Instant::now();
    let decision = forward_decision(mailbox, message, reason);
    log_decision(
        logger,
        run_id,
        message,
        &decision,
        "context_limit",
        started.elapsed(),
    )
    .await;
    record_agent_decision_for_history(processing, logger, run_id, message, &decision).await;
    let action = decision_with_runtime_fields(mailbox, message, &decision).action;
    send_and_mark_seen(
        mailbox,
        logger,
        run_id,
        mail,
        processing,
        message,
        action,
        "forwarded",
    )
    .await;
}
