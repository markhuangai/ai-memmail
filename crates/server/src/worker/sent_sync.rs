const SENT_SYNC_BATCH_SIZE: usize = 200;

async fn sync_sent_mailbox(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    processing: &dyn ProcessingStore,
) -> bool {
    if mailbox.imap.sent_backfill_days == 0 {
        return true;
    }
    let started = Instant::now();
    let state = match processing.sent_sync_state(&mailbox.id).await {
        Ok(state) => state,
        Err(error) => {
            log_sent_sync_failure(
                logger,
                run_id,
                mailbox,
                "sent_sync_state",
                started.elapsed(),
                error.to_string(),
            )
            .await;
            return false;
        }
    };
    let cutoff =
        chrono::Utc::now().timestamp() - i64::from(mailbox.imap.sent_backfill_days) * 24 * 60 * 60;
    let batch = match mail
        .fetch_sent(
            mailbox,
            state.as_ref().map(|state| &state.cursor),
            cutoff,
            SENT_SYNC_BATCH_SIZE,
        )
        .await
    {
        Ok(batch) => batch,
        Err(error) => {
            log_sent_sync_failure(
                logger,
                run_id,
                mailbox,
                "imap_sent_sync",
                started.elapsed(),
                error.to_string(),
            )
            .await;
            return false;
        }
    };
    if let Err(error) = processing
        .record_sent_batch(&mailbox.id, cutoff, &batch)
        .await
    {
        log_sent_sync_failure(
            logger,
            run_id,
            mailbox,
            "sent_sync_persist",
            started.elapsed(),
            error.to_string(),
        )
        .await;
        return false;
    }
    logger
        .log(mailbox_event(
            LogLevel::Info,
            run_id,
            &mailbox.id,
            "imap_sent_sync",
            if batch.complete {
                "complete"
            } else {
                "backfilling"
            },
            started.elapsed(),
            Some(format!(
                "folder={} messages={}",
                batch.folder_name,
                batch.messages.len()
            )),
        ))
        .await;
    batch.complete
}

async fn log_sent_sync_failure(
    logger: &dyn ActionLogger,
    run_id: &str,
    mailbox: &MailboxConfig,
    action: &'static str,
    duration: Duration,
    detail: String,
) {
    logger
        .log(mailbox_event(
            LogLevel::Error,
            run_id,
            &mailbox.id,
            action,
            "failed_closed",
            duration,
            Some(detail),
        ))
        .await;
}
