async fn claim_before_side_effect(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    mailbox: &MailboxConfig,
    message: &InboundMessage,
) -> bool {
    let started = Instant::now();
    match processing.claim_message(run_id, message).await {
        Ok(ProcessingClaim::Claimed) => {
            logger
                .log(message_event(
                    LogLevel::Debug,
                    run_id,
                    message,
                    "processing_claim",
                    "claimed",
                    started.elapsed(),
                    None,
                ))
                .await;
            true
        }
        Ok(ProcessingClaim::AlreadyFinished { status }) => {
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "processing_claim",
                    "dedupe_skip",
                    started.elapsed(),
                    Some(status),
                ))
                .await;
            mark_seen(mailbox, logger, run_id, mail, message, "dedupe_skip").await;
            false
        }
        Ok(ProcessingClaim::InProgress { status }) => {
            logger
                .log(message_event(
                    LogLevel::Warn,
                    run_id,
                    message,
                    "processing_claim",
                    "in_progress",
                    started.elapsed(),
                    Some(status),
                ))
                .await;
            false
        }
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "processing_claim",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            false
        }
    }
}

async fn update_processing_status(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    status: &str,
    outbound_action: Option<&OutboundActionKind>,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .update_message_status(&message.metadata.dedupe_key(), status, outbound_action)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "processing_update",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn mark_seen(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    message: &InboundMessage,
    status: &'static str,
) {
    let started = Instant::now();
    match mail.mark_seen(mailbox, message.metadata.uid).await {
        Ok(()) => {
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "imap_mark_seen",
                    status,
                    started.elapsed(),
                    None,
                ))
                .await;
        }
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "imap_mark_seen",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
        }
    }
}

fn safety_forward_action(
    mailbox: &MailboxConfig,
    message: &InboundMessage,
    reason: &str,
) -> OutboundAction {
    let intro = suspicious_forward_intro(reason, &message.metadata.from_addr);
    OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: mailbox.safety_forward_to.clone(),
        subject: suspicious_forward_subject(&message.metadata.subject),
        body: forward_body(&intro, message),
        reason: reason.to_string(),
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    }
}

fn outbound_review_forward_action(
    mailbox: &MailboxConfig,
    message: &InboundMessage,
    proposed: &OutboundAction,
    review: &OutboundReviewDecision,
) -> OutboundAction {
    let recipients = if mailbox.agent.default_forward_to.is_empty() {
        mailbox.safety_forward_to.clone()
    } else {
        mailbox.agent.default_forward_to.clone()
    };
    let intro = format!(
        "ai-memmail outbound review rejected a proposed {:?} for message from {}.\n\nReason: {}\n\nProposed recipients: {}\nProposed subject: {}\nProposed reason: {}\n\nThe original message is forwarded below for human review.",
        proposed.kind,
        message.metadata.from_addr,
        review.reason,
        proposed.recipients.join(", "),
        proposed.subject,
        proposed.reason
    );
    OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients,
        subject: format!("Fwd: {}", message.metadata.subject),
        body: forward_body(&intro, message),
        reason: format!("outbound review rejected: {}", review.reason),
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    }
}

fn mailbox_event(
    level: LogLevel,
    run_id: &str,
    mailbox_id: &str,
    action: impl Into<String>,
    status: impl Into<String>,
    duration: Duration,
    detail: Option<String>,
) -> ActionEvent {
    let mut event = action_event(level, run_id, action, status, duration);
    event.mailbox_id = Some(mailbox_id.to_string());
    event.detail = detail;
    event
}

fn message_event(
    level: LogLevel,
    run_id: &str,
    message: &InboundMessage,
    action: impl Into<String>,
    status: impl Into<String>,
    duration: Duration,
    detail: Option<String>,
) -> ActionEvent {
    let mut event = mailbox_event(
        level,
        run_id,
        &message.metadata.mailbox_id,
        action,
        status,
        duration,
        detail,
    );
    event.message_uid_validity = Some(message.metadata.uid_validity);
    event.message_uid = Some(message.metadata.uid);
    event
}
