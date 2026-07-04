fn decision_with_runtime_fields(
    mailbox: &MailboxConfig,
    message: &InboundMessage,
    decision: &AgentDecision,
) -> AgentDecision {
    AgentDecision {
        action: action_with_runtime_fields(mailbox, message, &decision.action),
        safety_notes: decision.safety_notes.clone(),
    }
}

fn action_with_runtime_fields(
    mailbox: &MailboxConfig,
    message: &InboundMessage,
    action: &OutboundAction,
) -> OutboundAction {
    match action.kind {
        OutboundActionKind::Forward => {
            let mut action = action.clone();
            let intro = action.body.trim().to_string();
            action.body = forward_body(&intro, message);
            action.message_id = None;
            action.in_reply_to = None;
            action.references.clear();
            action
        }
        OutboundActionKind::Reply => {
            let mut action = action.clone();
            action.body = automated_reply_body(&action.body);
            action.message_id = Some(outbound_message_id(mailbox));
            action.in_reply_to = message.metadata.message_id.clone();
            action.references = reply_references(&message.metadata);
            action
        }
        OutboundActionKind::Noop => {
            let mut action = action.clone();
            action.message_id = None;
            action.in_reply_to = None;
            action.references.clear();
            action
        }
    }
}

fn outbound_message_id(mailbox: &MailboxConfig) -> String {
    let domain = mailbox
        .smtp
        .from
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim())
        .filter(|domain| !domain.is_empty())
        .unwrap_or("ai-memmail.local");
    format!("<{}@{}>", Uuid::new_v4(), domain)
}

async fn reviewed_outbound_action(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    decision: &AgentDecision,
) -> Option<OutboundAction> {
    if !config.ai.review.enabled {
        return Some(decision.action.clone());
    }

    let started = Instant::now();
    match decisions
        .outbound_review(config, mailbox, message, decision)
        .await
    {
        Ok(review) if review.approved => {
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "outbound_review",
                    "approved",
                    started.elapsed(),
                    Some(review.reason.clone()),
                ))
                .await;
            record_outbound_review_for_history(
                processing,
                logger,
                run_id,
                message,
                "approved",
                &review.reason,
            )
            .await;
            Some(decision.action.clone())
        }
        Ok(review) => {
            logger
                .log(message_event(
                    LogLevel::Warn,
                    run_id,
                    message,
                    "outbound_review",
                    "rejected",
                    started.elapsed(),
                    Some(review.reason.clone()),
                ))
                .await;
            record_outbound_review_for_history(
                processing,
                logger,
                run_id,
                message,
                "rejected",
                &review.reason,
            )
            .await;
            Some(outbound_review_forward_action(
                mailbox,
                message,
                &decision.action,
                &review,
            ))
        }
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "outbound_review",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            record_outbound_review_for_history(
                processing,
                logger,
                run_id,
                message,
                "failed",
                &error.to_string(),
            )
            .await;
            None
        }
    }
}

fn outbound_status(kind: &OutboundActionKind) -> &'static str {
    match kind {
        OutboundActionKind::Reply => "replied",
        OutboundActionKind::Forward => "forwarded",
        OutboundActionKind::Noop => "noop",
    }
}

async fn send_and_mark_seen(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    action: OutboundAction,
    status: &'static str,
) {
    let started = Instant::now();
    record_outbound_action_for_history(processing, logger, run_id, message, &action).await;
    match mail.send(&mailbox.smtp, &action).await {
        Ok(()) => {
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "smtp_send",
                    status,
                    started.elapsed(),
                    Some(action.reason),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                message,
                status,
                Some(&action.kind),
            )
            .await;
            mark_seen(mailbox, logger, run_id, mail, message, status).await;
        }
        Err(error) => {
            update_processing_status(
                processing,
                logger,
                run_id,
                message,
                PROCESSING_STATUS_SEND_FAILED,
                Some(&action.kind),
            )
            .await;
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "smtp_send",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
        }
    }
}
