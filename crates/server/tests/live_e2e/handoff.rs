use std::time::Duration;

use ai_memmail_server::config::MailboxConfig;
use ai_memmail_server::mail::{
    reply_recipient, InboundMessage, LiveMailTransport, MailTransport, AUTOMATED_REPLY_NOTICE,
};
use ai_memmail_server::storage::{PgStore, ProcessedEmail, ProcessingStore};

#[derive(Debug, Clone)]
pub(super) struct LiveHandoffExpectation {
    pub(super) destination: String,
    pub(super) remote_target: String,
}

pub(super) async fn verify_ui_thread_handoff(
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    known_subject: &str,
) -> Option<LiveHandoffExpectation> {
    let handoff_mailbox = handoff_mailbox(forward);

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
        "derived live handoff destination must not be the monitored mailbox"
    );
    assert!(
        !destination.eq_ignore_ascii_case(&remote_target),
        "derived live handoff destination must differ from the remote sender mailbox"
    );

    let handoff = wait_for_active_handoff(
        processing,
        known_subject,
        &source.mailbox_id,
        &source.thread_id,
        &destination,
        &remote_target,
    )
    .await;
    assert_eq!(handoff.destination, destination);
    assert_eq!(handoff.remote_target, remote_target);

    let message = wait_for_handoff_mail(&handoff_mailbox, transport, known_subject, |message| {
        message.metadata.subject.contains(known_subject)
            && message
                .plain_text
                .contains("---------- Conversation handoff ---------")
            && message
                .plain_text
                .contains("According to configured MCP memory")
            && message.plain_text.contains(AUTOMATED_REPLY_NOTICE)
            && message.plain_text.contains("escalation to human")
    })
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

async fn wait_for_active_handoff(
    store: &PgStore,
    subject: &str,
    mailbox_id: &str,
    thread_id: &str,
    destination: &str,
    remote_target: &str,
) -> ai_memmail_server::storage::ThreadHandoff {
    let timeout = std::env::var("AI_MEMMAIL_LIVE_E2E_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(420);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let handoff = store
            .active_thread_handoff(mailbox_id, thread_id)
            .await
            .expect("load live UI handoff");
        if let Some(handoff) = handoff {
            if handoff.state == "active"
                && handoff.destination == destination
                && handoff.remote_target == remote_target
            {
                return handoff;
            }
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for UI-created handoff state; subject={subject}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn wait_for_handoff_mail(
    mailbox: &MailboxConfig,
    transport: &LiveMailTransport,
    subject: &str,
    matches: impl Fn(&InboundMessage) -> bool,
) -> InboundMessage {
    let timeout = std::env::var("AI_MEMMAIL_LIVE_E2E_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(420);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let messages = transport
            .fetch_unseen(mailbox, 200)
            .await
            .expect("fetch live handoff mailbox");
        for message in messages {
            if matches(&message) {
                transport
                    .mark_seen(mailbox, message.metadata.uid)
                    .await
                    .expect("mark live handoff message seen");
                return message;
            }
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for UI-created handoff mail; subject={subject}"
        );
        tokio::time::sleep(Duration::from_secs(8)).await;
    }
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

fn handoff_mailbox(forward: &MailboxConfig) -> MailboxConfig {
    let handoff_address = handoff_alias(&forward.address);
    let mut handoff = forward.clone();
    handoff.id = "live-e2e-handoff".to_string();
    handoff.address = handoff_address.clone();
    handoff.enabled = false;
    handoff.smtp.from = handoff_address;
    handoff
}

fn handoff_alias(address: &str) -> String {
    let address = reply_recipient(address);
    let (local, domain) = address
        .rsplit_once('@')
        .expect("forward mailbox address should include a domain");
    let local = local.split_once('+').map_or(local, |(base, _)| base);
    format!("{local}+handoff@{domain}")
}
