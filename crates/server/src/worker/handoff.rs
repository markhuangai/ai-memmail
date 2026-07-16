async fn route_active_thread_handoff(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    thread_context: &ThreadContext,
    handoff: &crate::storage::ThreadHandoff,
) {
    let destination = reply_recipient(&handoff.destination);
    let sender = reply_recipient(&message.metadata.from_addr);
    if sender.eq_ignore_ascii_case(&destination) {
        logger
            .log(message_event(
                LogLevel::Warn,
                run_id,
                message,
                "thread_handoff",
                "loop_prevented",
                Duration::default(),
                Some(format!("sender matches handoff destination {destination}")),
            ))
            .await;
        update_processing_status(
            processing,
            logger,
            run_id,
            message,
            PROCESSING_STATUS_HANDED_OFF,
            None,
        )
        .await;
        mark_seen(mailbox, logger, run_id, mail, message, "handoff_loop").await;
        return;
    }

    let handoff_context = handoff_context_with_current(thread_context, message);
    let action = match thread_handoff_action(mailbox, &handoff_context, handoff) {
        Ok(action) => action,
        Err(error) => {
            forward_context_limit_message(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                message,
                &error.to_string(),
            )
            .await;
            return;
        }
    };
    let delivery = NewThreadHandoffDelivery {
        request_id: Uuid::new_v4(),
        mailbox_id: mailbox.id.clone(),
        thread_id: thread_context.thread_id.clone(),
        source_run_id: uuid::Uuid::parse_str(run_id).ok(),
        destination: destination.clone(),
        remote_target: handoff.remote_target.clone(),
        outbound_message_id: action.message_id.clone().unwrap_or_default(),
    };
    if let Err(error) = processing.begin_thread_handoff_delivery(&delivery).await {
        log_retryable_message_error(
            logger,
            processing,
            run_id,
            message,
            "thread_handoff",
            Instant::now(),
            error.to_string(),
        )
        .await;
        return;
    }

    let started = Instant::now();
    record_outbound_action_for_history(processing, logger, run_id, message, &action).await;
    match mail.send(&mailbox.smtp, &action).await {
        Ok(()) => {
            let _ = processing
                .finish_thread_handoff_delivery(
                    &delivery.mailbox_id,
                    &delivery.thread_id,
                    delivery.request_id,
                    "sent",
                    None,
                )
                .await;
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "thread_handoff",
                    "sent",
                    started.elapsed(),
                    Some(format!("destination={destination}")),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                message,
                PROCESSING_STATUS_HANDED_OFF,
                Some(&OutboundActionKind::Forward),
            )
            .await;
            mark_seen(mailbox, logger, run_id, mail, message, PROCESSING_STATUS_HANDED_OFF).await;
        }
        Err(error) => {
            let detail = error.to_string();
            let _ = processing
                .finish_thread_handoff_delivery(
                    &delivery.mailbox_id,
                    &delivery.thread_id,
                    delivery.request_id,
                    "failed",
                    Some(&detail),
                )
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                message,
                PROCESSING_STATUS_SEND_FAILED,
                Some(&OutboundActionKind::Forward),
            )
            .await;
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "thread_handoff",
                    "failed",
                    started.elapsed(),
                    Some(detail),
                ))
                .await;
        }
    }
}

fn thread_handoff_action(
    mailbox: &MailboxConfig,
    thread_context: &ThreadContext,
    handoff: &crate::storage::ThreadHandoff,
) -> Result<OutboundAction, MailError> {
    let latest = thread_context
        .messages
        .iter()
        .rev()
        .find(|message| message.direction == MessageDirection::Inbound)
        .ok_or_else(|| MailError::Build("thread handoff has no inbound message".to_string()))?;
    let mut references = latest.references.clone();
    if let Some(message_id) = &latest.message_id {
        if !references.iter().any(|reference| reference == message_id) {
            references.push(message_id.clone());
        }
    }
    Ok(OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: vec![handoff.destination.clone()],
        subject: latest.subject.clone(),
        body: thread_handoff_body(thread_context)?,
        reason: "thread handed off for manual handling".to_string(),
        reply_to: Some(handoff.remote_target.clone()),
        message_id: Some(outbound_message_id(mailbox)),
        in_reply_to: latest.message_id.clone(),
        references,
    })
}

fn handoff_context_with_current(
    thread_context: &ThreadContext,
    message: &InboundMessage,
) -> ThreadContext {
    let mut context = thread_context.clone();
    context.messages.push(ThreadMessage {
        direction: MessageDirection::Inbound,
        message_id: message.metadata.message_id.clone(),
        in_reply_to: message.metadata.in_reply_to.clone(),
        references: message.metadata.references.clone(),
        from_addr: message.metadata.from_addr.clone(),
        recipients: message.metadata.recipients.clone(),
        subject: message.metadata.subject.clone(),
        authored_text: message.plain_text.clone(),
        body_truncated: message.plain_text.chars().count() > INBOUND_BODY_STORAGE_MAX_CHARS,
        timestamp: chrono::Utc::now().timestamp(),
    });
    context
}
