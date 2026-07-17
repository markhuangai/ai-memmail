use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ai_memmail_server::config::{
    AppConfig, BannedSenderConfig, BannedSenderKind, DatabaseConfig, MailboxConfig,
};
use ai_memmail_server::logging::{FanoutLogger, MemoryLogger};
use ai_memmail_server::mail::{
    InboundMessage, LiveMailTransport, MailTransport, OutboundAction, OutboundActionKind,
    AUTOMATED_REPLY_NOTICE,
};
use ai_memmail_server::storage::PgStore;
use ai_memmail_server::worker;

#[path = "live_e2e/handoff.rs"]
mod handoff;
use handoff::{verify_ui_thread_handoff, LiveHandoffExpectation};

const DEFAULT_CONFIG_PATH: &str = "config/config.yaml";

#[tokio::test(flavor = "multi_thread")]
async fn live_email_processing_scenarios() {
    if std::env::var("AI_MEMMAIL_LIVE_E2E").as_deref() != Ok("1") {
        eprintln!("skipping live e2e; set AI_MEMMAIL_LIVE_E2E=1");
        return;
    }

    let config_path = live_config_path();
    let mut config = AppConfig::load(&config_path).expect("load live config");
    normalize_live_paths(&mut config);
    config.validate().expect("validate live config");
    let monitored = config
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.enabled)
        .expect("at least one enabled mailbox");
    let forward = forward_mailbox(monitored);
    let transport = LiveMailTransport::default();
    let processing = live_processing_store(&config.database).await;
    let run_id = unique_run_id();
    let known_subject = format!("live-e2e known mcp {run_id}");

    if std::env::var("AI_MEMMAIL_LIVE_E2E_VERIFY_UI_HANDOFF").as_deref() == Ok("1") {
        verify_ui_thread_handoff(monitored, &forward, &transport, &processing, &known_subject)
            .await;
        return;
    }

    let (known_subject, known_reply_message_id) = run_known_mcp_reply(
        &config,
        monitored,
        &forward,
        &transport,
        &processing,
        &run_id,
    )
    .await;
    let escalation_subject = run_escalation_followup(
        &config,
        monitored,
        &forward,
        &transport,
        &processing,
        &known_subject,
        &known_reply_message_id,
    )
    .await;
    let subjects = vec![
        known_subject.clone(),
        escalation_subject.clone(),
        run_human_forward(
            &config,
            monitored,
            &forward,
            &transport,
            &processing,
            &run_id,
        )
        .await,
        run_quarantine_forward(
            &config,
            monitored,
            &forward,
            &transport,
            &processing,
            &run_id,
        )
        .await,
        run_banned_sender_forward(
            &config,
            monitored,
            &forward,
            &transport,
            &processing,
            &run_id,
        )
        .await,
    ];
    assert_processed_history(
        &processing,
        &subjects,
        &known_subject,
        &escalation_subject,
        None,
    )
    .await;
}

async fn run_known_mcp_reply(
    config: &AppConfig,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    run_id: &str,
) -> (String, String) {
    let subject = format!("live-e2e known mcp {run_id}");
    send_probe(
        transport,
        monitored,
        forward,
        &subject,
        "According to configured MCP memory, what minimum unit test coverage percentage is required for both the Rust backend and React frontend? Reply with the percentage and do not forward unless you cannot answer from MCP memory.",
    )
    .await;
    let message = wait_for_forward_mail(
        config,
        forward,
        transport,
        processing,
        &subject,
        |message| message.metadata.subject.contains(&subject) && message.plain_text.contains("90"),
    )
    .await;
    assert!(
        message.plain_text.contains("90"),
        "known MCP reply did not include expected coverage percentage; subject={}",
        message.metadata.subject
    );
    assert!(
        message.plain_text.contains(AUTOMATED_REPLY_NOTICE),
        "known MCP reply did not include automated reply notice"
    );
    let reply_message_id = message
        .metadata
        .message_id
        .clone()
        .expect("known MCP reply should have a message id");
    (subject, reply_message_id)
}

async fn run_escalation_followup(
    config: &AppConfig,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    known_subject: &str,
    known_reply_message_id: &str,
) -> String {
    let subject = format!("Re: {known_subject}");
    let probe_body = "escalation to human";
    transport
        .send(
            &forward.smtp,
            &OutboundAction {
                kind: OutboundActionKind::Reply,
                recipients: vec![monitored.address.clone()],
                subject: subject.clone(),
                body: probe_body.to_string(),
                reason: "live e2e escalation follow-up".to_string(),
                reply_to: None,
                message_id: None,
                in_reply_to: Some(known_reply_message_id.to_string()),
                references: vec![known_reply_message_id.to_string()],
            },
        )
        .await
        .expect("send live escalation follow-up");
    let message = wait_for_forward_mail(
        config,
        forward,
        transport,
        processing,
        &subject,
        |message| {
            message.metadata.subject.contains(&subject)
                && message.metadata.subject.starts_with("Fwd:")
                && message.plain_text.contains("Human review requested")
                && message.plain_text.contains(probe_body)
        },
    )
    .await;
    assert_forward_contains_original(&message, &subject, probe_body);
    subject
}

async fn run_human_forward(
    config: &AppConfig,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    run_id: &str,
) -> String {
    let subject = format!("live-e2e human forward {run_id}");
    let probe_body = "Please forward this to a human for manual review. Do not answer directly.";
    send_probe(transport, monitored, forward, &subject, probe_body).await;
    let message = wait_for_forward_mail(
        config,
        forward,
        transport,
        processing,
        &subject,
        |message| {
            message.metadata.subject.contains(&subject)
                && message.metadata.subject.starts_with("Fwd:")
                && message.plain_text.contains("Human review requested")
        },
    )
    .await;
    assert_forward_contains_original(&message, &subject, probe_body);
    subject
}

async fn run_quarantine_forward(
    config: &AppConfig,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    run_id: &str,
) -> String {
    let subject = format!("live-e2e quarantine {run_id}");
    let probe_body = "Live E2E quarantine probe: this message intentionally contains the keyword jailbreak so the deterministic safety precheck routes it to human review.";
    send_probe(transport, monitored, forward, &subject, probe_body).await;
    let message = wait_for_forward_mail(
        config,
        forward,
        transport,
        processing,
        &subject,
        |message| {
            message.metadata.subject.contains(&subject)
                && message
                    .metadata
                    .subject
                    .starts_with("[Potential jailbreak]")
                && message.plain_text.contains("quarantined")
        },
    )
    .await;
    assert_forward_contains_original(&message, &subject, probe_body);
    subject
}

async fn run_banned_sender_forward(
    config: &AppConfig,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    run_id: &str,
) -> String {
    let mut config = config.clone();
    config.banned_senders.push(BannedSenderConfig {
        kind: BannedSenderKind::Email,
        value: forward.address.clone(),
        reason: "live e2e banned sender route".to_string(),
    });
    let subject = format!("live-e2e banned sender {run_id}");
    let probe_body =
        "This routine message should be forwarded because the test config bans the sender.";
    send_probe(transport, monitored, forward, &subject, probe_body).await;
    let message = wait_for_forward_mail(
        &config,
        forward,
        transport,
        processing,
        &subject,
        |message| {
            message.metadata.subject.contains(&subject)
                && message
                    .metadata
                    .subject
                    .starts_with("[Potential jailbreak]")
                && message
                    .plain_text
                    .contains("sender is on the banned sender list")
        },
    )
    .await;
    assert_forward_contains_original(&message, &subject, probe_body);
    subject
}

fn assert_forward_contains_original(message: &InboundMessage, subject: &str, original_body: &str) {
    assert!(
        message
            .plain_text
            .contains("---------- Forwarded message ---------"),
        "forwarded body should include the forwarded-message marker; subject={subject}"
    );
    assert!(
        message.plain_text.contains(&format!("Subject: {subject}")),
        "forwarded body should include original subject metadata; subject={subject}"
    );
    assert!(
        message.plain_text.contains(original_body),
        "forwarded body should include the original email body; subject={subject}"
    );
}

async fn send_probe(
    transport: &LiveMailTransport,
    monitored: &MailboxConfig,
    forward: &MailboxConfig,
    subject: &str,
    body: &str,
) {
    transport
        .send(
            &forward.smtp,
            &OutboundAction {
                kind: OutboundActionKind::Forward,
                recipients: vec![monitored.address.clone()],
                subject: subject.to_string(),
                body: body.to_string(),
                reason: "live e2e probe".to_string(),
                reply_to: None,
                message_id: None,
                in_reply_to: None,
                references: vec![],
            },
        )
        .await
        .expect("send live probe");
}

async fn wait_for_forward_mail(
    config: &AppConfig,
    forward: &MailboxConfig,
    transport: &LiveMailTransport,
    processing: &PgStore,
    subject: &str,
    matches: impl Fn(&InboundMessage) -> bool,
) -> InboundMessage {
    let timeout = std::env::var("AI_MEMMAIL_LIVE_E2E_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(420);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    let mut last_events: Vec<String>;
    loop {
        let logger = MemoryLogger::default();
        let logger_with_history = FanoutLogger::new(&logger, processing);
        worker::run_once_with_processing_store(
            config,
            &logger_with_history,
            "live-e2e",
            processing,
        )
        .await;
        last_events = logger
            .events()
            .into_iter()
            .map(|event| {
                format!(
                    "level={:?} mailbox={:?} uid={:?} action={} status={} detail={}",
                    event.level,
                    event.mailbox_id,
                    event.message_uid,
                    event.action,
                    event.status,
                    event.detail.unwrap_or_default()
                )
            })
            .collect();

        let messages = transport
            .fetch_unseen(forward, 200)
            .await
            .expect("fetch forward mailbox");
        for message in messages {
            if matches(&message) {
                transport
                    .mark_seen(forward, message.metadata.uid)
                    .await
                    .expect("mark forward response seen");
                return message;
            }
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for live e2e result for subject {subject}; last worker events: {:?}",
            last_events
        );
        tokio::time::sleep(Duration::from_secs(8)).await;
    }
}

async fn assert_processed_history(
    store: &PgStore,
    subjects: &[String],
    known_subject: &str,
    escalation_subject: &str,
    handoff: Option<&LiveHandoffExpectation>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let messages = store
            .list_processed_emails(100)
            .await
            .expect("list processed email history");
        let missing: Vec<&String> = subjects
            .iter()
            .filter(|subject| {
                !messages
                    .iter()
                    .any(|message| message.subject.as_str() == subject.as_str())
            })
            .collect();
        if missing.is_empty() {
            let known = messages
                .iter()
                .find(|message| message.subject == known_subject)
                .expect("known MCP history row");
            assert_eq!(known.outbound_action.as_deref(), Some("reply"));
            assert!(
                known
                    .inbound_body
                    .as_deref()
                    .unwrap_or_default()
                    .contains("According to configured MCP memory"),
                "known MCP history row should include inbound body"
            );
            assert!(
                known
                    .outbound_body
                    .as_deref()
                    .unwrap_or_default()
                    .contains("90"),
                "known MCP history row should include the reply body"
            );
            assert!(
                known
                    .outbound_body
                    .as_deref()
                    .unwrap_or_default()
                    .contains(AUTOMATED_REPLY_NOTICE),
                "known MCP history row should include automated reply notice"
            );
            assert!(
                known.outbound_message_id.is_some(),
                "known MCP history row should include outbound message id"
            );
            assert!(
                known.logs.iter().any(|entry| entry.action == "smtp_send"),
                "known MCP history row should include SMTP timeline logs"
            );
            if let Some(expected) = handoff {
                let handoff = known
                    .handoff
                    .as_ref()
                    .expect("known thread handoff summary");
                assert_eq!(handoff.state, "active");
                assert_eq!(handoff.destination, expected.destination);
                assert_eq!(handoff.remote_target, expected.remote_target);
                assert!(
                    known
                        .logs
                        .iter()
                        .any(|entry| entry.action == "thread_handoff" && entry.status == "sent"),
                    "known MCP history row should include handoff timeline logs"
                );
            }

            let escalation = messages
                .iter()
                .find(|message| message.subject == escalation_subject)
                .expect("escalation history row");
            assert_eq!(escalation.outbound_action.as_deref(), Some("forward"));
            assert_eq!(escalation.thread_id, known.thread_id);
            assert!(
                escalation
                    .inbound_body
                    .as_deref()
                    .unwrap_or_default()
                    .contains("escalation to human"),
                "escalation history row should include inbound body"
            );

            for subject in &subjects[1..] {
                let forwarded = messages
                    .iter()
                    .find(|message| message.subject == *subject)
                    .expect("forward history row");
                assert_eq!(forwarded.outbound_action.as_deref(), Some("forward"));
                assert!(
                    forwarded.outbound_body_redacted,
                    "forward history row should redact stored body for subject {subject}"
                );
            }
            return;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for processed history rows; missing={missing:?}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn forward_mailbox(monitored: &MailboxConfig) -> MailboxConfig {
    let forward_address = monitored
        .agent
        .default_forward_to
        .first()
        .or_else(|| monitored.safety_forward_to.first())
        .expect("mailbox default_forward_to or safety_forward_to")
        .clone();
    let mut forward = monitored.clone();
    forward.id = "live-e2e-forward".to_string();
    forward.address = forward_address.clone();
    forward.enabled = false;
    forward.imap.username = forward_address.clone();
    forward.imap.password = monitored.imap.password.clone();
    forward.smtp.username = forward_address.clone();
    forward.smtp.password = monitored.smtp.password.clone();
    forward.smtp.from = forward_address;
    forward
}

fn unique_run_id() -> String {
    if let Ok(run_id) = std::env::var("AI_MEMMAIL_LIVE_E2E_RUN_ID") {
        return run_id;
    }
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_millis();
    format!("{millis}-{}", std::process::id())
}

async fn live_processing_store(config: &DatabaseConfig) -> PgStore {
    let mut database = config.clone();
    database.host = std::env::var("AI_MEMMAIL_LIVE_E2E_DB_HOST").unwrap_or_else(|_| {
        if database.host == "postgres" {
            "127.0.0.1".to_string()
        } else {
            database.host.clone()
        }
    });
    database.port = std::env::var("AI_MEMMAIL_LIVE_E2E_DB_PORT")
        .ok()
        .map(|value| {
            value
                .parse()
                .expect("AI_MEMMAIL_LIVE_E2E_DB_PORT is a port")
        })
        .unwrap_or_else(|| {
            if database.host == "127.0.0.1" && database.port == 5432 {
                15432
            } else {
                database.port
            }
        });
    let store = PgStore::connect(&database)
        .await
        .expect("connect live e2e postgres");
    store.migrate().await.expect("migrate live e2e postgres");
    store
}

fn live_config_path() -> PathBuf {
    let configured = PathBuf::from(
        std::env::var("AI_MEMMAIL_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string()),
    );
    if configured.is_absolute() || configured.exists() {
        return configured;
    }
    workspace_root().join(configured)
}

fn normalize_live_paths(config: &mut AppConfig) {
    if config.prompts.root.is_relative() && !config.prompts.root.exists() {
        config.prompts.root = workspace_root().join(&config.prompts.root);
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("server crate lives under workspace/crates/server")
}
