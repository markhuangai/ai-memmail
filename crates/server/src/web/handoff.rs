async fn create_handoff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Json(request): Json<HandoffRequest>,
) -> Result<Json<HandoffResponse>, ApiError> {
    require_auth(&state, &headers)?;
    tracing::info!(run_id = %run_id, request_id = %request.request_id, action = "thread_handoff", status = "started", "thread handoff requested");
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
    let action = thread_handoff_action(mailbox, &thread_context, &destination, &remote_target)
        .map_err(|error| {
            tracing::warn!(run_id = %run_id, action = "thread_handoff", status = "rejected", %error, "thread handoff rejected");
            ApiError::from_mail_build(error)
        })?;
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
    let event = ActionEvent {
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
    };
    StdoutLogger.log(event.clone()).await;
    store
        .insert_action_log(&event)
        .await
        .map_err(ApiError::from_storage)
}
