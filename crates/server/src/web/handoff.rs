async fn create_handoff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Json(request): Json<HandoffRequest>,
) -> Result<Json<HandoffResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    let source = store
        .thread_handoff_source(&run_id)
        .await
        .map_err(ApiError::from_storage)?;
    let mailbox = config
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.id == source.mailbox_id)
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("mailbox {} is no longer configured", source.mailbox_id),
        })?;
    let destination = validate_handoff_destination(mailbox, &request.destination)?;
    store
        .validate_thread_handoff_ready(&source.mailbox_id, &source.thread_id)
        .await
        .map_err(ApiError::from_storage)?;
    let remote_target = reply_recipient(
        &store
            .latest_thread_remote_target(&source.mailbox_id, &source.thread_id)
            .await
            .map_err(ApiError::from_storage)?,
    );
    validate_handoff_target(&destination, &remote_target)?;
    let thread_context = store
        .load_thread_context_by_id(mailbox, &source.thread_id)
        .await
        .map_err(ApiError::from_storage)?;
    let action = handoff_action(mailbox, &thread_context, &destination, &remote_target)
        .map_err(ApiError::from_mail_build)?;
    let delivery = store
        .begin_thread_handoff_delivery(&NewThreadHandoffDelivery {
            request_id: request.request_id,
            mailbox_id: source.mailbox_id.clone(),
            thread_id: source.thread_id.clone(),
            source_run_id: Some(source.run_id),
            destination: destination.clone(),
            remote_target: remote_target.clone(),
            outbound_message_id: action.message_id.clone().unwrap_or_default(),
        })
        .await
        .map_err(ApiError::from_storage)?;
    if delivery.status == "sent" {
        return Ok(Json(HandoffResponse {
            handoff: handoff_summary(&store, &source).await?,
        }));
    }

    match state.mail.send(&mailbox.smtp, &action).await {
        Ok(()) => {
            store
                .finish_thread_handoff_delivery(
                    &source.mailbox_id,
                    &source.thread_id,
                    request.request_id,
                    "sent",
                    None,
                )
                .await
                .map_err(ApiError::from_storage)?;
            log_handoff_event(
                &store,
                &source,
                "sent",
                Some(format!("destination={destination}")),
            )
            .await?;
            Ok(Json(HandoffResponse {
                handoff: handoff_summary(&store, &source).await?,
            }))
        }
        Err(error) => {
            let detail = error.to_string();
            store
                .finish_thread_handoff_delivery(
                    &source.mailbox_id,
                    &source.thread_id,
                    request.request_id,
                    "failed",
                    Some(&detail),
                )
                .await
                .map_err(ApiError::from_storage)?;
            log_handoff_event(&store, &source, "failed", Some(detail.clone())).await?;
            Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: detail,
            })
        }
    }
}

fn validate_handoff_destination(
    mailbox: &MailboxConfig,
    destination: &str,
) -> Result<String, ApiError> {
    let parsed = destination
        .trim()
        .parse::<LettreMailbox>()
        .map_err(|error| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("handoff destination must be one valid email address: {error}"),
        })?;
    let destination = parsed.email.to_string();
    if destination.eq_ignore_ascii_case(&reply_recipient(&mailbox.address))
        || destination.eq_ignore_ascii_case(&reply_recipient(&mailbox.smtp.from))
    {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "handoff destination must not be the managed mailbox".to_string(),
        });
    }
    Ok(destination)
}

fn validate_handoff_target(destination: &str, remote_target: &str) -> Result<(), ApiError> {
    if destination.eq_ignore_ascii_case(remote_target) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "handoff destination must not be the remote sender".to_string(),
        });
    }
    Ok(())
}

fn handoff_action(
    mailbox: &MailboxConfig,
    thread_context: &ThreadContext,
    destination: &str,
    remote_target: &str,
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
        recipients: vec![destination.to_string()],
        subject: latest.subject.clone(),
        body: thread_handoff_body(thread_context)?,
        html_body: None,
        reason: "thread handed off for manual handling".to_string(),
        reply_to: Some(remote_target.to_string()),
        message_id: Some(outbound_message_id(mailbox)),
        in_reply_to: latest.message_id.clone(),
        references,
    })
}

async fn handoff_summary(
    store: &PgStore,
    source: &ThreadHandoffSource,
) -> Result<Option<ThreadHandoffSummary>, ApiError> {
    Ok(store
        .active_thread_handoff(&source.mailbox_id, &source.thread_id)
        .await
        .map_err(ApiError::from_storage)?
        .map(|handoff| ThreadHandoffSummary {
            state: handoff.state,
            destination: handoff.destination,
            remote_target: handoff.remote_target,
            last_error: handoff.last_error,
            updated_at: handoff.updated_at,
        }))
}

async fn log_handoff_event(
    store: &PgStore,
    source: &ThreadHandoffSource,
    status: &str,
    detail: Option<String>,
) -> Result<(), ApiError> {
    store
        .insert_action_log(&ActionEvent {
            level: if status == "sent" {
                LogLevel::Info
            } else {
                LogLevel::Error
            },
            run_id: source.run_id.to_string(),
            mailbox_id: Some(source.mailbox_id.clone()),
            message_uid_validity: Some(source.uid_validity),
            message_uid: Some(source.uid),
            action: "thread_handoff".to_string(),
            status: status.to_string(),
            duration_ms: 0,
            detail,
        })
        .await
        .map_err(ApiError::from_storage)
}
