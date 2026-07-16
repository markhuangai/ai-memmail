use std::time::Duration;

use ai_memmail_server::config::{AppConfig, MailboxConfig};
use ai_memmail_server::logging::{ActionEvent, LogLevel};
use ai_memmail_server::mail::{
    outbound_message_id, reply_recipient, thread_handoff_body, LiveMailTransport, MailTransport,
    MessageDirection, OutboundAction, OutboundActionKind, AUTOMATED_REPLY_NOTICE,
};
use ai_memmail_server::storage::{
    NewThreadHandoffDelivery, PgStore, ProcessedEmail, ProcessingStore,
};
use uuid::Uuid;

use super::wait_for_forward_mail;

#[derive(Debug, Clone)]
pub(super) struct LiveHandoffExpectation {
    pub(super) destination: String,
    pub(super) remote_target: String,
}

pub(super) async fn run_thread_handoff(
    config: &AppConfig,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    known_subject: &str,
) -> Option<LiveHandoffExpectation> {
    let Some(handoff_mailbox) = handoff_mailbox(monitored, forward) else {
        eprintln!(
            "skipping live handoff delivery; set AI_MEMMAIL_LIVE_HANDOFF_TO to an accessible mailbox distinct from the monitored and remote sender addresses"
        );
        return None;
    };

    let source_row = wait_for_history_row(processing, known_subject).await;
    let source = processing
        .thread_handoff_source(&source_row.run_id)
        .await
        .expect("load live handoff source");
    processing
        .validate_thread_handoff_ready(&source.mailbox_id, &source.thread_id)
        .await
        .expect("validate live handoff source thread");
    let remote_target = reply_recipient(
        &processing
            .latest_thread_remote_target(&source.mailbox_id, &source.thread_id)
            .await
            .expect("load live handoff remote target"),
    );
    let destination = reply_recipient(&handoff_mailbox.address);
    assert!(
        !destination.eq_ignore_ascii_case(&reply_recipient(&monitored.address)),
        "AI_MEMMAIL_LIVE_HANDOFF_TO must not be the monitored mailbox"
    );
    assert!(
        !destination.eq_ignore_ascii_case(&remote_target),
        "AI_MEMMAIL_LIVE_HANDOFF_TO must differ from the remote sender mailbox"
    );

    let thread_context = processing
        .load_thread_context_by_id(monitored, &source.thread_id)
        .await
        .expect("load live handoff thread context");
    let latest = thread_context
        .messages
        .iter()
        .rev()
        .find(|message| message.direction == MessageDirection::Inbound)
        .expect("live handoff thread has an inbound message");
    let mut references = latest.references.clone();
    if let Some(message_id) = &latest.message_id {
        if !references.iter().any(|reference| reference == message_id) {
            references.push(message_id.clone());
        }
    }
    let action = OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: vec![destination.clone()],
        subject: latest.subject.clone(),
        body: thread_handoff_body(&thread_context).expect("build live handoff thread body"),
        reason: "live e2e thread handoff".to_string(),
        reply_to: Some(remote_target.clone()),
        message_id: Some(outbound_message_id(monitored)),
        in_reply_to: latest.message_id.clone(),
        references,
    };
    let request_id = Uuid::new_v4();
    let delivery = NewThreadHandoffDelivery {
        request_id,
        mailbox_id: source.mailbox_id.clone(),
        thread_id: source.thread_id.clone(),
        source_run_id: Some(source.run_id),
        destination: destination.clone(),
        remote_target: remote_target.clone(),
        outbound_message_id: action
            .message_id
            .clone()
            .expect("live handoff action message id"),
    };
    let started = processing
        .begin_thread_handoff_delivery(&delivery)
        .await
        .expect("begin live handoff delivery");
    assert_eq!(started.status, "sending");
    match transport.send(&monitored.smtp, &action).await {
        Ok(()) => {
            processing
                .finish_thread_handoff_delivery(
                    &source.mailbox_id,
                    &source.thread_id,
                    request_id,
                    "sent",
                    None,
                )
                .await
                .expect("finish live handoff delivery");
            processing
                .insert_action_log(&ActionEvent {
                    level: LogLevel::Info,
                    run_id: source.run_id.to_string(),
                    mailbox_id: Some(source.mailbox_id.clone()),
                    message_uid_validity: Some(source.uid_validity),
                    message_uid: Some(source.uid),
                    action: "thread_handoff".to_string(),
                    status: "sent".to_string(),
                    duration_ms: 0,
                    detail: Some(format!("destination={destination}")),
                })
                .await
                .expect("record live handoff log");
        }
        Err(error) => {
            let detail = error.to_string();
            processing
                .finish_thread_handoff_delivery(
                    &source.mailbox_id,
                    &source.thread_id,
                    request_id,
                    "failed",
                    Some(&detail),
                )
                .await
                .expect("finish failed live handoff delivery");
            panic!("send live handoff delivery failed: {detail}");
        }
    }

    let message = wait_for_forward_mail(
        config,
        &handoff_mailbox,
        transport,
        processing,
        known_subject,
        |message| {
            message.metadata.subject.contains(known_subject)
                && message
                    .plain_text
                    .contains("---------- Conversation handoff ---------")
                && message
                    .plain_text
                    .contains("According to configured MCP memory")
                && message.plain_text.contains(AUTOMATED_REPLY_NOTICE)
                && message.plain_text.contains("escalation to human")
        },
    )
    .await;
    assert!(
        message.plain_text.contains("90"),
        "handoff body should include the automated reply content"
    );

    Some(LiveHandoffExpectation {
        destination,
        remote_target,
    })
}

async fn wait_for_history_row(store: &PgStore, subject: &str) -> ProcessedEmail {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(message) = store
            .list_processed_emails(100)
            .await
            .expect("list processed email history")
            .into_iter()
            .find(|message| message.subject == subject)
        {
            return message;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for processed history row; subject={subject}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn handoff_mailbox(monitored: &MailboxConfig, forward: &MailboxConfig) -> Option<MailboxConfig> {
    let handoff_address = match std::env::var("AI_MEMMAIL_LIVE_HANDOFF_TO") {
        Ok(address) if !address.trim().is_empty() => reply_recipient(&address),
        _ => return None,
    };
    let mut handoff = monitored.clone();
    handoff.id = "live-e2e-handoff".to_string();
    handoff.address = handoff_address.clone();
    handoff.enabled = false;
    handoff.imap.username = std::env::var("AI_MEMMAIL_LIVE_HANDOFF_IMAP_USERNAME")
        .unwrap_or_else(|_| handoff_address.clone());
    handoff.imap.password = std::env::var("AI_MEMMAIL_LIVE_HANDOFF_IMAP_PASSWORD")
        .unwrap_or_else(|_| forward.imap.password.clone());
    handoff.smtp.username = handoff_address.clone();
    handoff.smtp.password = forward.smtp.password.clone();
    handoff.smtp.from = handoff_address;
    Some(handoff)
}
