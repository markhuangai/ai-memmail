use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::ai::{
    forward_decision, human_review_requested, AgentDecision, AiError, DecisionEngine,
    LiveDecisionEngine, OutboundReviewDecision,
};
use crate::classification::{EmailClassification, EmailRule, ResolvedEmailClassification};
use crate::config::{AppConfig, ConfigError, MailboxConfig};
use crate::logging::{
    action_event, ActionEvent, ActionLogger, FanoutLogger, LogLevel, StdoutLogger,
};
use crate::mail::{
    apply_reply_signature, forward_body, message_matches_accepted_conditions, reply_recipient,
    reply_references, thread_handoff_body, InboundMessage, LiveMailTransport, MailError,
    MailTransport, MessageDirection, OutboundAction, OutboundActionKind, ThreadContext,
    ThreadMessage,
};
use crate::safety::{
    decide, sender_is_banned, suspicious_forward_intro, suspicious_forward_subject, SafetyDecision,
    SafetyDisposition, SafetyScanResult,
};
use crate::storage::{
    MemoryProcessingStore, NewThreadHandoffDelivery, PgStore, ProcessingClaim, ProcessingStore,
    INBOUND_BODY_STORAGE_MAX_CHARS, PROCESSING_STATUS_HANDED_OFF,
    PROCESSING_STATUS_RETRYABLE_FAILED, PROCESSING_STATUS_SEND_FAILED,
};

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Storage(#[from] crate::storage::StorageError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxPollPlan {
    pub mailbox_id: String,
    pub interval: Duration,
    pub mcp_server_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderPrecheck {
    Allowed,
    Banned { reason: String },
}

pub async fn run(config_path: PathBuf) -> Result<(), WorkerError> {
    let stdout_logger = StdoutLogger;
    let mail = LiveMailTransport::default();
    let decisions = LiveDecisionEngine::default();
    let initial_config = AppConfig::load(&config_path)?;
    initial_config.validate()?;
    let processing = PgStore::connect(&initial_config.database).await?;
    let logger = FanoutLogger::new(&stdout_logger, &processing);
    loop {
        let started = Instant::now();
        let config = AppConfig::load(&config_path)?;
        config.validate()?;
        let run_id = Uuid::new_v4().to_string();
        run_once_with_store(&config, &logger, &run_id, &mail, &decisions, &processing).await;
        let sleep_for = next_poll_delay(&config);
        logger
            .log(action_event(
                LogLevel::Debug,
                run_id,
                "worker_sleep",
                format!("sleeping_{}s", sleep_for.as_secs()),
                started.elapsed(),
            ))
            .await;
        tokio::select! {
            _ = tokio::time::sleep(sleep_for) => {}
            result = tokio::signal::ctrl_c() => {
                if result.is_ok() {
                    break;
                }
            }
        }
    }
    Ok(())
}

pub async fn run_once(config: &AppConfig, logger: &dyn ActionLogger, run_id: &str) {
    let mail = LiveMailTransport::default();
    let decisions = LiveDecisionEngine::default();
    let processing = MemoryProcessingStore::default();
    run_once_with_store(config, logger, run_id, &mail, &decisions, &processing).await;
}

pub async fn run_once_with_processing_store(
    config: &AppConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    processing: &dyn ProcessingStore,
) {
    let mail = LiveMailTransport::default();
    let decisions = LiveDecisionEngine::default();
    run_once_with_store(config, logger, run_id, &mail, &decisions, processing).await;
}

pub async fn run_once_with(
    config: &AppConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
) {
    let processing = MemoryProcessingStore::default();
    run_once_with_store(config, logger, run_id, mail, decisions, &processing).await;
}

pub async fn run_once_with_store(
    config: &AppConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
) {
    let started = Instant::now();
    let plans = poll_plans(config);
    logger
        .log(action_event(
            LogLevel::Info,
            run_id,
            "worker_poll_plan",
            format!("mailboxes={}", plans.len()),
            started.elapsed(),
        ))
        .await;

    if let Err(error) = processing
        .ensure_default_classification_policy(config)
        .await
    {
        logger
            .log(action_event(
                LogLevel::Error,
                run_id,
                "classification_policy_seed",
                "failed",
                started.elapsed(),
            ))
            .await;
        tracing::error!(%error, "failed to seed classification policy");
        return;
    }

    for mailbox in config.mailboxes.iter().filter(|mailbox| mailbox.enabled) {
        if sync_sent_mailbox(mailbox, logger, run_id, mail, processing).await {
            process_mailbox(config, mailbox, logger, run_id, mail, decisions, processing).await;
        }
    }
}

pub fn load_and_plan(config_path: &Path) -> Result<Vec<MailboxPollPlan>, ConfigError> {
    let config = AppConfig::load(config_path)?;
    config.validate()?;
    Ok(poll_plans(&config))
}

pub fn poll_plans(config: &AppConfig) -> Vec<MailboxPollPlan> {
    config
        .mailboxes
        .iter()
        .filter(|mailbox| mailbox.enabled)
        .map(mailbox_poll_plan)
        .collect()
}

pub fn mailbox_poll_plan(mailbox: &MailboxConfig) -> MailboxPollPlan {
    MailboxPollPlan {
        mailbox_id: mailbox.id.clone(),
        interval: Duration::from_secs(mailbox.poll_interval_seconds),
        mcp_server_count: mailbox.mcp_servers.len(),
    }
}

pub fn next_poll_delay(config: &AppConfig) -> Duration {
    poll_plans(config)
        .into_iter()
        .map(|plan| plan.interval)
        .min()
        .unwrap_or_else(|| Duration::from_secs(60))
}

pub fn precheck_sender(sender: &str, config: &AppConfig) -> SenderPrecheck {
    if sender_is_banned(sender, &config.banned_senders) {
        SenderPrecheck::Banned {
            reason: "sender is on the banned sender list".to_string(),
        }
    } else {
        SenderPrecheck::Allowed
    }
}

pub fn should_forward_for_human_review(decision: &SafetyDecision) -> bool {
    decision.disposition == SafetyDisposition::QuarantineAndForward
}

include!("worker/step_timeout.rs");

include!("worker/sent_sync.rs");

include!("worker/thread_context.rs");

include!("worker/classification.rs");

include!("worker/handoff.rs");

include!("worker/process.rs");

include!("worker/outbound.rs");

include!("worker/history.rs");

include!("worker/processing.rs");

#[cfg(test)]
mod tests;
