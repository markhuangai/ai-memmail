use std::path::{Path, PathBuf};
use std::time::Duration;

use ai_memmail_server::config::AppConfig;
use ai_memmail_server::mail::{
    LiveMailTransport, MailTransport, OutboundAction, OutboundActionKind,
};
use uuid::Uuid;

const DEFAULT_CONFIG_PATH: &str = "config/config.yaml";

#[tokio::test(flavor = "multi_thread")]
async fn inspect_live_sent_mailbox_without_processing() {
    if std::env::var("AI_MEMMAIL_LIVE_SENT_INSPECT").as_deref() != Ok("1") {
        eprintln!("skipping Sent inspection; set AI_MEMMAIL_LIVE_SENT_INSPECT=1");
        return;
    }

    let config = AppConfig::load(&live_config_path()).expect("load live config");
    config.validate().expect("validate live config");
    let mailbox = config
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.enabled)
        .expect("at least one enabled mailbox");
    let cutoff = chrono::Utc::now().timestamp() - 30 * 24 * 60 * 60;
    let batch = LiveMailTransport::default()
        .fetch_sent(mailbox, None, cutoff, 10)
        .await
        .expect("inspect live Sent mailbox");

    eprintln!(
        "LIVE_SENT_INSPECTED folder={} uid_validity={} sampled_messages={} complete={}",
        batch.folder_name,
        batch.uid_validity,
        batch.messages.len(),
        batch.complete
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_existing_human_reply_without_sending() {
    let Ok(message_id) = std::env::var("AI_MEMMAIL_LIVE_EXISTING_MESSAGE_ID") else {
        eprintln!("skipping existing reply capture; set AI_MEMMAIL_LIVE_EXISTING_MESSAGE_ID");
        return;
    };
    let context_token = std::env::var("AI_MEMMAIL_LIVE_EXISTING_CONTEXT_TOKEN")
        .expect("AI_MEMMAIL_LIVE_EXISTING_CONTEXT_TOKEN is required");
    let config = AppConfig::load(&live_config_path()).expect("load live config");
    config.validate().expect("validate live config");
    let mailbox = config
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.enabled)
        .expect("at least one enabled mailbox");
    let transport = LiveMailTransport::default();
    let reply = wait_for_reply(mailbox, &transport, &message_id).await;

    inspect_and_mark_reply(mailbox, &transport, reply, &message_id, &context_token).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_human_client_reply_without_ai_processing() {
    if std::env::var("AI_MEMMAIL_LIVE_HUMAN_E2E").as_deref() != Ok("1") {
        eprintln!("skipping human reply probe; set AI_MEMMAIL_LIVE_HUMAN_E2E=1");
        return;
    }

    let recipient = std::env::var("AI_MEMMAIL_LIVE_RECIPIENT")
        .expect("AI_MEMMAIL_LIVE_RECIPIENT is required for the human reply probe");
    let config = AppConfig::load(&live_config_path()).expect("load live config");
    config.validate().expect("validate live config");
    let mailbox = config
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.enabled)
        .expect("at least one enabled mailbox");
    let transport = LiveMailTransport::default();
    let probe_id = Uuid::new_v4();
    let message_id = format!("<human-thread-probe-{probe_id}@markhuang.ai>");
    let subject = format!("ai-memmail human thread probe {probe_id}");
    let context_token = format!("THREAD-PROBE-{probe_id}");
    let body = format!(
        "This is an authorized ai-memmail thread-context probe.\n\n\
         Context token: {context_token}\n\
         Please reply normally above the quoted message and leave the quoted text intact."
    );

    transport
        .send(
            &mailbox.smtp,
            &OutboundAction {
                kind: OutboundActionKind::Forward,
                recipients: vec![recipient],
                subject: subject.clone(),
                body,
                html_body: None,
                reason: "authorized human reply context probe".to_string(),
                reply_to: None,
                message_id: Some(message_id.clone()),
                in_reply_to: None,
                references: vec![],
            },
        )
        .await
        .expect("send human reply probe");

    eprintln!("HUMAN_REPLY_PROBE_SENT subject={subject} message_id={message_id}");
    let reply = wait_for_reply(mailbox, &transport, &message_id).await;
    inspect_and_mark_reply(mailbox, &transport, reply, &message_id, &context_token).await;
}

async fn inspect_and_mark_reply(
    mailbox: &ai_memmail_server::config::MailboxConfig,
    transport: &LiveMailTransport,
    reply: ai_memmail_server::mail::InboundMessage,
    message_id: &str,
    context_token: &str,
) {
    let quoted_original = reply.plain_text.contains(&context_token);
    eprintln!(
        "HUMAN_REPLY_PROBE_CAPTURED message_id={} in_reply_to={} references={} thread_id={} body_chars={} quoted_original={quoted_original}",
        reply.metadata.message_id.as_deref().unwrap_or("(none)"),
        reply.metadata.in_reply_to.as_deref().unwrap_or("(none)"),
        reply.metadata.references.len(),
        reply.metadata.thread_id(),
        reply.plain_text.chars().count(),
    );
    transport
        .mark_seen(mailbox, reply.metadata.uid)
        .await
        .expect("mark captured human reply seen");
    assert_eq!(reply.metadata.in_reply_to.as_deref(), Some(message_id));
    assert!(
        quoted_original,
        "human mail client omitted the probe's quoted text"
    );
}

async fn wait_for_reply(
    mailbox: &ai_memmail_server::config::MailboxConfig,
    transport: &LiveMailTransport,
    original_message_id: &str,
) -> ai_memmail_server::mail::InboundMessage {
    let timeout = std::env::var("AI_MEMMAIL_LIVE_HUMAN_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(900);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let messages = transport
            .fetch_unseen(mailbox, 200)
            .await
            .expect("fetch unseen human replies");
        if let Some(reply) = messages.into_iter().find(|message| {
            message.metadata.in_reply_to.as_deref() == Some(original_message_id)
                || message
                    .metadata
                    .references
                    .iter()
                    .any(|reference| reference == original_message_id)
        }) {
            return reply;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for a reply to {original_message_id}"
        );
        tokio::time::sleep(Duration::from_secs(8)).await;
    }
}

fn live_config_path() -> PathBuf {
    let configured = PathBuf::from(
        std::env::var("AI_MEMMAIL_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string()),
    );
    if configured.is_absolute() || configured.exists() {
        configured
    } else {
        workspace_root().join(configured)
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("server crate lives under workspace/crates/server")
}
