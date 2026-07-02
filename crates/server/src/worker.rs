use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::ai::{AgentDecision, DecisionEngine, LiveDecisionEngine, OutboundReviewDecision};
use crate::config::{AppConfig, ConfigError, MailboxConfig};
use crate::logging::{
    action_event, ActionEvent, ActionLogger, FanoutLogger, LogLevel, StdoutLogger,
};
use crate::mail::{
    forward_body, InboundMessage, LiveMailTransport, MailTransport, OutboundAction,
    OutboundActionKind,
};
use crate::safety::{
    decide, sender_is_banned, suspicious_forward_intro, suspicious_forward_subject, SafetyDecision,
    SafetyDisposition, SafetyScanResult,
};
use crate::storage::{
    MemoryProcessingStore, PgStore, ProcessingClaim, ProcessingStore,
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

    for mailbox in config.mailboxes.iter().filter(|mailbox| mailbox.enabled) {
        process_mailbox(config, mailbox, logger, run_id, mail, decisions, processing).await;
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

async fn process_mailbox(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
) {
    let started = Instant::now();
    match mail.fetch_unseen(mailbox, 10).await {
        Ok(messages) => {
            logger
                .log(mailbox_event(
                    LogLevel::Info,
                    run_id,
                    &mailbox.id,
                    "imap_fetch",
                    format!("messages={}", messages.len()),
                    started.elapsed(),
                    None,
                ))
                .await;
            for message in messages {
                let message_run_id = Uuid::new_v4().to_string();
                process_message(
                    config,
                    mailbox,
                    logger,
                    &message_run_id,
                    mail,
                    decisions,
                    processing,
                    message,
                )
                .await;
            }
        }
        Err(error) => {
            logger
                .log(mailbox_event(
                    LogLevel::Error,
                    run_id,
                    &mailbox.id,
                    "imap_fetch",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
        }
    }
}

async fn process_message(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: InboundMessage,
) {
    let started = Instant::now();
    match precheck_sender(&message.metadata.from_addr, config) {
        SenderPrecheck::Banned { reason } => {
            let action = safety_forward_action(mailbox, &message, &reason);
            if !claim_before_side_effect(processing, logger, run_id, mail, mailbox, &message).await
            {
                return;
            }
            send_and_mark_seen(
                mailbox,
                logger,
                run_id,
                mail,
                processing,
                &message,
                action,
                "banned_sender",
            )
            .await;
            return;
        }
        SenderPrecheck::Allowed => {}
    }

    if !claim_before_side_effect(processing, logger, run_id, mail, mailbox, &message).await {
        return;
    }

    let scan = match decisions.safety_scan(config, mailbox, &message).await {
        Ok(scan) => scan,
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    &message,
                    "safety_scan",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
    };
    log_safety_scan_result(logger, run_id, &message, &scan, started.elapsed()).await;
    record_safety_result_for_history(processing, logger, run_id, &message, &scan).await;
    let safety_decision = decide(&scan);
    if should_forward_for_human_review(&safety_decision) {
        let action = safety_forward_action(mailbox, &message, &safety_decision.reason);
        if !record_quarantine_state(
            processing,
            logger,
            run_id,
            &message,
            &scan,
            &safety_decision,
        )
        .await
        {
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
        send_and_mark_seen(
            mailbox,
            logger,
            run_id,
            mail,
            processing,
            &message,
            action,
            "quarantined",
        )
        .await;
        return;
    }

    let decision = match decisions.agent_decision(config, mailbox, &message).await {
        Ok(decision) => decision,
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    &message,
                    "agent_decision",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                PROCESSING_STATUS_RETRYABLE_FAILED,
                None,
            )
            .await;
            return;
        }
    };
    log_agent_decision(logger, run_id, &message, &decision, started.elapsed()).await;
    record_agent_decision_for_history(processing, logger, run_id, &message, &decision).await;
    let outbound_decision = decision_with_forward_body(&message, &decision);

    match &outbound_decision.action.kind {
        OutboundActionKind::Noop => {
            record_outbound_action_for_history(
                processing,
                logger,
                run_id,
                &message,
                &outbound_decision.action,
            )
            .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                &message,
                "noop",
                Some(&OutboundActionKind::Noop),
            )
            .await;
            mark_seen(mailbox, logger, run_id, mail, &message, "noop").await;
        }
        OutboundActionKind::Reply | OutboundActionKind::Forward => {
            let action = match reviewed_outbound_action(
                config,
                mailbox,
                logger,
                run_id,
                decisions,
                processing,
                &message,
                &outbound_decision,
            )
            .await
            {
                Some(action) => action,
                None => {
                    update_processing_status(
                        processing,
                        logger,
                        run_id,
                        &message,
                        PROCESSING_STATUS_RETRYABLE_FAILED,
                        None,
                    )
                    .await;
                    return;
                }
            };
            let status = outbound_status(&action.kind);
            send_and_mark_seen(
                mailbox, logger, run_id, mail, processing, &message, action, status,
            )
            .await;
        }
    }
}

fn decision_with_forward_body(message: &InboundMessage, decision: &AgentDecision) -> AgentDecision {
    AgentDecision {
        action: action_with_forward_body(message, &decision.action),
        safety_notes: decision.safety_notes.clone(),
    }
}

fn action_with_forward_body(message: &InboundMessage, action: &OutboundAction) -> OutboundAction {
    if action.kind != OutboundActionKind::Forward {
        return action.clone();
    }

    let mut action = action.clone();
    let intro = action.body.trim().to_string();
    action.body = forward_body(&intro, message);
    action
}

async fn reviewed_outbound_action(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    decisions: &dyn DecisionEngine,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    decision: &AgentDecision,
) -> Option<OutboundAction> {
    if !config.ai.review.enabled {
        return Some(decision.action.clone());
    }

    let started = Instant::now();
    match decisions
        .outbound_review(config, mailbox, message, decision)
        .await
    {
        Ok(review) if review.approved => {
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "outbound_review",
                    "approved",
                    started.elapsed(),
                    Some(review.reason.clone()),
                ))
                .await;
            record_outbound_review_for_history(
                processing,
                logger,
                run_id,
                message,
                "approved",
                &review.reason,
            )
            .await;
            Some(decision.action.clone())
        }
        Ok(review) => {
            logger
                .log(message_event(
                    LogLevel::Warn,
                    run_id,
                    message,
                    "outbound_review",
                    "rejected",
                    started.elapsed(),
                    Some(review.reason.clone()),
                ))
                .await;
            record_outbound_review_for_history(
                processing,
                logger,
                run_id,
                message,
                "rejected",
                &review.reason,
            )
            .await;
            Some(outbound_review_forward_action(
                mailbox,
                message,
                &decision.action,
                &review,
            ))
        }
        Err(error) => {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "outbound_review",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            record_outbound_review_for_history(
                processing,
                logger,
                run_id,
                message,
                "failed",
                &error.to_string(),
            )
            .await;
            None
        }
    }
}

fn outbound_status(kind: &OutboundActionKind) -> &'static str {
    match kind {
        OutboundActionKind::Reply => "replied",
        OutboundActionKind::Forward => "forwarded",
        OutboundActionKind::Noop => "noop",
    }
}

async fn send_and_mark_seen(
    mailbox: &MailboxConfig,
    logger: &dyn ActionLogger,
    run_id: &str,
    mail: &dyn MailTransport,
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    action: OutboundAction,
    status: &'static str,
) {
    let started = Instant::now();
    record_outbound_action_for_history(processing, logger, run_id, message, &action).await;
    match mail.send(&mailbox.smtp, &action).await {
        Ok(()) => {
            logger
                .log(message_event(
                    LogLevel::Info,
                    run_id,
                    message,
                    "smtp_send",
                    status,
                    started.elapsed(),
                    Some(action.reason),
                ))
                .await;
            update_processing_status(
                processing,
                logger,
                run_id,
                message,
                status,
                Some(&action.kind),
            )
            .await;
            mark_seen(mailbox, logger, run_id, mail, message, status).await;
        }
        Err(error) => {
            update_processing_status(
                processing,
                logger,
                run_id,
                message,
                PROCESSING_STATUS_SEND_FAILED,
                Some(&action.kind),
            )
            .await;
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "smtp_send",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
        }
    }
}

async fn log_safety_scan_result(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    scan: &SafetyScanResult,
    duration: Duration,
) {
    logger
        .log(message_event(
            LogLevel::Info,
            run_id,
            message,
            "safety_scan",
            crate::storage::safety_category_value(&scan.category),
            duration,
            Some(scan.reason.clone()),
        ))
        .await;
}

async fn log_agent_decision(
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    decision: &AgentDecision,
    duration: Duration,
) {
    logger
        .log(message_event(
            LogLevel::Info,
            run_id,
            message,
            "agent_decision",
            crate::storage::outbound_action_value(&decision.action.kind),
            duration,
            Some(decision.action.reason.clone()),
        ))
        .await;
}

async fn record_safety_result_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    scan: &SafetyScanResult,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_safety_result(&message.metadata.dedupe_key(), &scan.category, &scan.reason)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "safety_result_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_agent_decision_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    decision: &AgentDecision,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_agent_decision(&message.metadata.dedupe_key(), decision)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "agent_decision_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_outbound_action_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    action: &OutboundAction,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_outbound_action(&message.metadata.dedupe_key(), action)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "outbound_action_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_outbound_review_for_history(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    status: &str,
    reason: &str,
) {
    let started = Instant::now();
    if let Err(error) = processing
        .record_outbound_review(&message.metadata.dedupe_key(), status, reason)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "outbound_review_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
    }
}

async fn record_quarantine_state(
    processing: &dyn ProcessingStore,
    logger: &dyn ActionLogger,
    run_id: &str,
    message: &InboundMessage,
    scan: &SafetyScanResult,
    decision: &SafetyDecision,
) -> bool {
    let started = Instant::now();
    let mut persisted = true;
    if let Err(error) = processing
        .record_safety_result(&message.metadata.dedupe_key(), &scan.category, &scan.reason)
        .await
    {
        logger
            .log(message_event(
                LogLevel::Error,
                run_id,
                message,
                "safety_result_persist",
                "failed",
                started.elapsed(),
                Some(error.to_string()),
            ))
            .await;
        persisted = false;
    }

    if decision.add_sender_to_review {
        let started = Instant::now();
        if let Err(error) = processing
            .upsert_sender_review(
                &message.metadata.from_addr,
                &message.metadata.mailbox_id,
                &decision.reason,
            )
            .await
        {
            logger
                .log(message_event(
                    LogLevel::Error,
                    run_id,
                    message,
                    "sender_review_persist",
                    "failed",
                    started.elapsed(),
                    Some(error.to_string()),
                ))
                .await;
            persisted = false;
        }
    }

    persisted
}

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use crate::ai::{AgentDecision, AiError};
    use crate::config::{
        AgentConfig, AiConfig, AiProtocol, BannedSenderConfig, BannedSenderKind, DatabaseConfig,
        ImapConfig, LoggingConfig, PromptConfig, ReviewConfig, SmtpConfig,
    };
    use crate::mail::{DedupeKey, MailError, MessageMetadata};
    use crate::safety::{SafetyCategory, SafetyScanResult};

    use super::*;

    fn config() -> AppConfig {
        AppConfig {
            version: 1,
            database: DatabaseConfig {
                host: "postgres".to_string(),
                port: 5432,
                username: "user".to_string(),
                password: "db-secret".to_string(),
                database: "ai_memmail".to_string(),
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "json".to_string(),
                verbose_actions: true,
                retention_days: 180,
            },
            prompts: PromptConfig {
                root: "prompts".into(),
                safety_scan: "safety.md".into(),
            },
            ai: AiConfig {
                protocol: AiProtocol::Openai,
                api_url: "https://api.example/v1".to_string(),
                api_secret: "secret".to_string(),
                model: "model".to_string(),
                review: ReviewConfig {
                    enabled: false,
                    prompt_path: "review.md".into(),
                },
            },
            mcp_servers: BTreeMap::new(),
            mailboxes: vec![MailboxConfig {
                id: "support".to_string(),
                address: "support@example.com".to_string(),
                enabled: true,
                poll_interval_seconds: 30,
                safety_forward_to: vec!["human@example.com".to_string()],
                mcp_servers: vec![],
                agent: AgentConfig {
                    system_prompt_path: "agent.md".into(),
                    default_forward_to: vec![],
                },
                imap: ImapConfig {
                    host: "imap.example.com".to_string(),
                    port: 993,
                    tls: true,
                    username: "support@example.com".to_string(),
                    password: "secret".to_string(),
                    folder: "INBOX".to_string(),
                },
                smtp: SmtpConfig {
                    host: "smtp.example.com".to_string(),
                    port: 587,
                    starttls: true,
                    username: "support@example.com".to_string(),
                    password: "secret".to_string(),
                    from: "support@example.com".to_string(),
                },
            }],
            banned_senders: vec![BannedSenderConfig {
                kind: BannedSenderKind::Domain,
                value: "blocked.test".to_string(),
                reason: "jailbreak attempts".to_string(),
            }],
        }
    }

    fn inbound(uid: u64, from_addr: &str, subject: &str, plain_text: &str) -> InboundMessage {
        InboundMessage {
            metadata: MessageMetadata {
                mailbox_id: "support".to_string(),
                uid_validity: 1,
                uid,
                message_id: Some(format!("<{uid}@example.com>")),
                from_addr: from_addr.to_string(),
                subject: subject.to_string(),
            },
            plain_text: plain_text.to_string(),
        }
    }

    struct FakeMail {
        messages: Mutex<Vec<InboundMessage>>,
        sent: Mutex<Vec<OutboundAction>>,
        seen: Mutex<Vec<DedupeKey>>,
        fail_fetch: bool,
        fail_send: bool,
        fail_mark_seen: bool,
    }

    impl FakeMail {
        fn new(messages: Vec<InboundMessage>) -> Self {
            Self {
                messages: Mutex::new(messages),
                sent: Mutex::new(Vec::new()),
                seen: Mutex::new(Vec::new()),
                fail_fetch: false,
                fail_send: false,
                fail_mark_seen: false,
            }
        }

        fn with_fail_fetch(mut self) -> Self {
            self.fail_fetch = true;
            self
        }

        fn with_fail_send(mut self) -> Self {
            self.fail_send = true;
            self
        }

        fn with_fail_mark_seen(mut self) -> Self {
            self.fail_mark_seen = true;
            self
        }

        fn sent(&self) -> Vec<OutboundAction> {
            self.sent.lock().expect("sent lock").clone()
        }

        fn seen(&self) -> Vec<DedupeKey> {
            self.seen.lock().expect("seen lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl MailTransport for FakeMail {
        async fn fetch_unseen(
            &self,
            _mailbox: &MailboxConfig,
            _limit: usize,
        ) -> Result<Vec<InboundMessage>, MailError> {
            if self.fail_fetch {
                return Err(MailError::Imap("fetch failed".to_string()));
            }
            Ok(std::mem::take(
                &mut *self.messages.lock().expect("messages lock"),
            ))
        }

        async fn send(&self, _smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
            if self.fail_send {
                return Err(MailError::Smtp("send failed".to_string()));
            }
            self.sent.lock().expect("sent lock").push(action.clone());
            Ok(())
        }

        async fn mark_seen(&self, mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
            if self.fail_mark_seen {
                return Err(MailError::Imap("mark seen failed".to_string()));
            }
            self.seen.lock().expect("seen lock").push(DedupeKey {
                mailbox_id: mailbox.id.clone(),
                uid_validity: 1,
                uid,
            });
            Ok(())
        }
    }

    struct FakeDecisionEngine {
        scan: SafetyScanResult,
        decision: AgentDecision,
        review: OutboundReviewDecision,
        fail_safety: bool,
        fail_agent: bool,
        fail_review: bool,
        calls: Arc<Mutex<DecisionCallCounts>>,
        reviewed_actions: Arc<Mutex<Vec<OutboundAction>>>,
    }

    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    struct DecisionCallCounts {
        safety_scan: usize,
        agent_decision: usize,
        outbound_review: usize,
    }

    impl FakeDecisionEngine {
        fn call_counts(&self) -> DecisionCallCounts {
            self.calls.lock().expect("decision calls lock").clone()
        }

        fn reviewed_actions(&self) -> Vec<OutboundAction> {
            self.reviewed_actions
                .lock()
                .expect("reviewed actions lock")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl DecisionEngine for FakeDecisionEngine {
        async fn safety_scan(
            &self,
            _config: &AppConfig,
            _mailbox: &MailboxConfig,
            _message: &InboundMessage,
        ) -> Result<SafetyScanResult, AiError> {
            self.calls.lock().expect("decision calls lock").safety_scan += 1;
            if self.fail_safety {
                return Err(AiError::Provider("safety failed".to_string()));
            }
            Ok(self.scan.clone())
        }

        async fn agent_decision(
            &self,
            _config: &AppConfig,
            _mailbox: &MailboxConfig,
            _message: &InboundMessage,
        ) -> Result<AgentDecision, AiError> {
            self.calls
                .lock()
                .expect("decision calls lock")
                .agent_decision += 1;
            if self.fail_agent {
                return Err(AiError::Provider("agent failed".to_string()));
            }
            Ok(self.decision.clone())
        }

        async fn outbound_review(
            &self,
            _config: &AppConfig,
            _mailbox: &MailboxConfig,
            _message: &InboundMessage,
            decision: &AgentDecision,
        ) -> Result<OutboundReviewDecision, AiError> {
            self.calls
                .lock()
                .expect("decision calls lock")
                .outbound_review += 1;
            self.reviewed_actions
                .lock()
                .expect("reviewed actions lock")
                .push(decision.action.clone());
            if self.fail_review {
                return Err(AiError::Provider("review failed".to_string()));
            }
            Ok(self.review.clone())
        }
    }

    enum FakeClaimOutcome {
        Claimed,
        InProgress,
        AlreadyFinished,
        Fail,
    }

    struct FakeProcessingStore {
        claims: Mutex<Vec<FakeClaimOutcome>>,
        run_ids: Mutex<Vec<String>>,
        fail_update: bool,
    }

    impl FakeProcessingStore {
        fn new(claims: Vec<FakeClaimOutcome>) -> Self {
            Self {
                claims: Mutex::new(claims),
                run_ids: Mutex::new(Vec::new()),
                fail_update: false,
            }
        }

        fn with_fail_update(mut self) -> Self {
            self.fail_update = true;
            self
        }

        fn run_ids(&self) -> Vec<String> {
            self.run_ids.lock().expect("run ids lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl ProcessingStore for FakeProcessingStore {
        async fn claim_message(
            &self,
            run_id: &str,
            _message: &InboundMessage,
        ) -> Result<ProcessingClaim, crate::storage::StorageError> {
            self.run_ids
                .lock()
                .map_err(|_| crate::storage::StorageError::LockPoisoned)?
                .push(run_id.to_string());
            let outcome = self
                .claims
                .lock()
                .map_err(|_| crate::storage::StorageError::LockPoisoned)?
                .remove(0);
            match outcome {
                FakeClaimOutcome::Claimed => Ok(ProcessingClaim::Claimed),
                FakeClaimOutcome::InProgress => Ok(ProcessingClaim::InProgress {
                    status: "processing".to_string(),
                }),
                FakeClaimOutcome::AlreadyFinished => Ok(ProcessingClaim::AlreadyFinished {
                    status: "replied".to_string(),
                }),
                FakeClaimOutcome::Fail => Err(crate::storage::StorageError::LockPoisoned),
            }
        }

        async fn update_message_status(
            &self,
            _key: &DedupeKey,
            _status: &str,
            _outbound_action: Option<&OutboundActionKind>,
        ) -> Result<(), crate::storage::StorageError> {
            if self.fail_update {
                return Err(crate::storage::StorageError::LockPoisoned);
            }
            Ok(())
        }

        async fn record_safety_result(
            &self,
            _key: &DedupeKey,
            _category: &SafetyCategory,
            _reason: &str,
        ) -> Result<(), crate::storage::StorageError> {
            Ok(())
        }

        async fn upsert_sender_review(
            &self,
            _sender: &str,
            _mailbox_id: &str,
            _reason: &str,
        ) -> Result<(), crate::storage::StorageError> {
            Ok(())
        }
    }

    fn fake_decisions(scan: SafetyScanResult, action: OutboundAction) -> FakeDecisionEngine {
        FakeDecisionEngine {
            scan,
            decision: AgentDecision {
                action,
                safety_notes: "tested".to_string(),
            },
            review: OutboundReviewDecision {
                approved: true,
                reason: "approved".to_string(),
            },
            fail_safety: false,
            fail_agent: false,
            fail_review: false,
            calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
            reviewed_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn rejecting_review_decisions() -> FakeDecisionEngine {
        FakeDecisionEngine {
            scan: safe_scan(),
            decision: AgentDecision {
                action: reply_action(),
                safety_notes: "tested".to_string(),
            },
            review: OutboundReviewDecision {
                approved: false,
                reason: "unexpected recipient".to_string(),
            },
            fail_safety: false,
            fail_agent: false,
            fail_review: false,
            calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
            reviewed_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing_review_decisions() -> FakeDecisionEngine {
        FakeDecisionEngine {
            scan: safe_scan(),
            decision: AgentDecision {
                action: reply_action(),
                safety_notes: "tested".to_string(),
            },
            review: OutboundReviewDecision {
                approved: true,
                reason: "approved".to_string(),
            },
            fail_safety: false,
            fail_agent: false,
            fail_review: true,
            calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
            reviewed_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing_safety_decisions() -> FakeDecisionEngine {
        FakeDecisionEngine {
            scan: safe_scan(),
            decision: AgentDecision {
                action: reply_action(),
                safety_notes: "tested".to_string(),
            },
            review: OutboundReviewDecision {
                approved: true,
                reason: "approved".to_string(),
            },
            fail_safety: true,
            fail_agent: false,
            fail_review: false,
            calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
            reviewed_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing_agent_decisions() -> FakeDecisionEngine {
        FakeDecisionEngine {
            scan: safe_scan(),
            decision: AgentDecision {
                action: reply_action(),
                safety_notes: "tested".to_string(),
            },
            review: OutboundReviewDecision {
                approved: true,
                reason: "approved".to_string(),
            },
            fail_safety: false,
            fail_agent: true,
            fail_review: false,
            calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
            reviewed_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn safe_scan() -> SafetyScanResult {
        SafetyScanResult {
            category: SafetyCategory::Safe,
            reason: "routine".to_string(),
            confidence: 0.9,
        }
    }

    fn reply_action() -> OutboundAction {
        OutboundAction {
            kind: OutboundActionKind::Reply,
            recipients: vec!["person@example.com".to_string()],
            subject: "Re: Hello".to_string(),
            body: "Known answer".to_string(),
            reason: "memory supported answer".to_string(),
        }
    }

    fn forward_action() -> OutboundAction {
        OutboundAction {
            kind: OutboundActionKind::Forward,
            recipients: vec!["human@example.com".to_string()],
            subject: "Fwd: cited agent memory".to_string(),
            body: "Josh asked whether a short meeting would be useful.".to_string(),
            reason: "requires Mark's judgment".to_string(),
        }
    }

    fn noop_action() -> OutboundAction {
        OutboundAction {
            kind: OutboundActionKind::Noop,
            recipients: vec![],
            subject: String::new(),
            body: String::new(),
            reason: "no safe action".to_string(),
        }
    }

    #[test]
    fn builds_poll_plan_for_enabled_mailboxes() {
        let plans = poll_plans(&config());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].mailbox_id, "support");
        assert_eq!(plans[0].interval, Duration::from_secs(30));
    }

    #[test]
    fn next_poll_delay_uses_shortest_enabled_mailbox_interval() {
        assert_eq!(next_poll_delay(&config()), Duration::from_secs(30));
    }

    #[test]
    fn next_poll_delay_defaults_when_no_mailboxes_are_enabled() {
        let mut config = config();
        config.mailboxes[0].enabled = false;
        assert_eq!(next_poll_delay(&config), Duration::from_secs(60));
    }

    #[test]
    fn sender_precheck_blocks_banned_domains() {
        assert_eq!(
            precheck_sender("person@blocked.test", &config()),
            SenderPrecheck::Banned {
                reason: "sender is on the banned sender list".to_string()
            }
        );
    }

    #[test]
    fn sender_precheck_allows_unlisted_senders() {
        assert_eq!(
            precheck_sender("person@example.com", &config()),
            SenderPrecheck::Allowed
        );
    }

    #[tokio::test]
    async fn run_once_logs_mailbox_count() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;
        let events = logger.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].action, "worker_poll_plan");
        assert_eq!(events[0].status, "mailboxes=1");
        assert_eq!(events[1].action, "imap_fetch");
        assert_eq!(events[1].status, "messages=0");
    }

    #[tokio::test]
    async fn safe_message_replies_and_marks_seen() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(42, "person@example.com", "Hello", "Question")]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        let sent = mail.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, OutboundActionKind::Reply);
        assert_eq!(mail.seen()[0].uid, 42);
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "smtp_send" && event.status == "replied"));
    }

    #[tokio::test]
    async fn agent_forward_includes_original_message_body_before_send() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(
            62,
            "Josh <joshua.kappler@gmail.com>",
            "cited agent memory",
            "Original memo-engine meeting request.",
        )]);
        let decisions = fake_decisions(safe_scan(), forward_action());

        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        let sent = mail.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, OutboundActionKind::Forward);
        assert_eq!(
            sent[0].body,
            "Josh asked whether a short meeting would be useful.\n\n---------- Forwarded message ---------\nFrom: Josh <joshua.kappler@gmail.com>\nSubject: cited agent memory\nMessage-ID: <62@example.com>\nUID: 1:62\n\nOriginal memo-engine meeting request."
        );
        assert_eq!(mail.seen()[0].uid, 62);
    }

    #[tokio::test]
    async fn messages_in_one_poll_get_distinct_run_ids() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![
            inbound(60, "first@example.com", "First", "Question"),
            inbound(61, "second@example.com", "Second", "Question"),
        ]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        let processing =
            FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed, FakeClaimOutcome::Claimed]);

        run_once_with_store(
            &config(),
            &logger,
            "poll-run",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        let run_ids = processing.run_ids();
        assert_eq!(run_ids.len(), 2);
        assert_ne!(run_ids[0], run_ids[1]);
        uuid::Uuid::parse_str(&run_ids[0]).unwrap();
        uuid::Uuid::parse_str(&run_ids[1]).unwrap();
    }

    #[tokio::test]
    async fn enabled_outbound_review_approves_reply_before_send() {
        let mut config = config();
        config.ai.review.enabled = true;
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(56, "person@example.com", "Hello", "Question")]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

        let sent = mail.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, OutboundActionKind::Reply);
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "outbound_review" && event.status == "approved"));
    }

    #[tokio::test]
    async fn outbound_review_receives_composed_forward_body() {
        let mut config = config();
        config.ai.review.enabled = true;
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(
            63,
            "Josh <joshua.kappler@gmail.com>",
            "cited agent memory",
            "Original memo-engine meeting request.",
        )]);
        let decisions = fake_decisions(safe_scan(), forward_action());

        run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

        let reviewed = decisions.reviewed_actions();
        assert_eq!(reviewed.len(), 1);
        assert_eq!(reviewed[0].kind, OutboundActionKind::Forward);
        assert_eq!(
            reviewed[0].body,
            "Josh asked whether a short meeting would be useful.\n\n---------- Forwarded message ---------\nFrom: Josh <joshua.kappler@gmail.com>\nSubject: cited agent memory\nMessage-ID: <63@example.com>\nUID: 1:63\n\nOriginal memo-engine meeting request."
        );
        assert_eq!(mail.sent()[0].body, reviewed[0].body);
    }

    #[tokio::test]
    async fn enabled_outbound_review_rejection_forwards_to_human_reviewer() {
        let mut config = config();
        config.ai.review.enabled = true;
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(57, "person@example.com", "Hello", "Question")]);
        let decisions = rejecting_review_decisions();
        run_once_with(&config, &logger, "run-test", &mail, &decisions).await;

        let sent = mail.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, OutboundActionKind::Forward);
        assert_eq!(sent[0].recipients, vec!["human@example.com"]);
        assert!(sent[0].reason.contains("outbound review rejected"));
        assert!(!sent[0].body.contains("Known answer"));
        assert_eq!(mail.seen()[0].uid, 57);
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "outbound_review" && event.status == "rejected"));
    }

    #[tokio::test]
    async fn enabled_outbound_review_failure_leaves_message_unseen() {
        let mut config = config();
        config.ai.review.enabled = true;
        let logger = crate::logging::MemoryLogger::default();
        let processing = MemoryProcessingStore::default();
        let message = inbound(58, "person@example.com", "Hello", "Question");
        let key = message.metadata.dedupe_key();
        let mail = FakeMail::new(vec![message]);
        let decisions = failing_review_decisions();
        run_once_with_store(&config, &logger, "run-test", &mail, &decisions, &processing).await;

        assert!(mail.sent().is_empty());
        assert!(mail.seen().is_empty());
        assert_eq!(
            processing.status(&key),
            Some(PROCESSING_STATUS_RETRYABLE_FAILED.to_string())
        );
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "outbound_review" && event.status == "failed"));
    }

    #[tokio::test]
    async fn unsafe_message_is_quarantined_and_forwarded() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(
            43,
            "person@example.com",
            "Policy override attempt",
            "Sensitive request",
        )]);
        let decisions = fake_decisions(
            SafetyScanResult {
                category: SafetyCategory::PromptInjection,
                reason: "tries to override policy".to_string(),
                confidence: 0.98,
            },
            reply_action(),
        );
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        let sent = mail.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, OutboundActionKind::Forward);
        assert!(sent[0].subject.starts_with("[Potential jailbreak]"));
        assert_eq!(mail.seen()[0].uid, 43);
    }

    #[tokio::test]
    async fn unsafe_message_persists_safety_and_sender_review_state() {
        let logger = crate::logging::MemoryLogger::default();
        let processing = MemoryProcessingStore::default();
        let message = inbound(
            59,
            "person@example.com",
            "Policy override attempt",
            "Sensitive request",
        );
        let key = message.metadata.dedupe_key();
        let mail = FakeMail::new(vec![message.clone()]);
        let decisions = fake_decisions(
            SafetyScanResult {
                category: SafetyCategory::PromptInjection,
                reason: "tries to override policy".to_string(),
                confidence: 0.98,
            },
            reply_action(),
        );
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert_eq!(
            processing.safety_result(&key),
            Some(crate::storage::StoredSafetyResult {
                category: "prompt_injection".to_string(),
                reason: "tries to override policy".to_string()
            })
        );
        assert_eq!(
            processing.sender_review("person@example.com"),
            Some(crate::storage::SenderReviewRecord {
                mailbox_id: "support".to_string(),
                reason: "tries to override policy".to_string()
            })
        );
        assert_eq!(mail.sent().len(), 1);
        assert_eq!(mail.seen()[0].uid, 59);
    }

    #[tokio::test]
    async fn banned_sender_is_forwarded_before_ai_processing() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(
            44,
            "person@blocked.test",
            "Routine",
            "Please answer",
        )]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        let sent = mail.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, OutboundActionKind::Forward);
        assert!(sent[0].body.contains("sender is on the banned sender list"));
        assert_eq!(mail.seen()[0].uid, 44);
    }

    #[tokio::test]
    async fn fetch_failure_is_logged() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![]).with_fail_fetch();
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "imap_fetch" && event.status == "failed"));
    }

    #[tokio::test]
    async fn safety_failure_leaves_message_unseen() {
        let logger = crate::logging::MemoryLogger::default();
        let processing = MemoryProcessingStore::default();
        let message = inbound(45, "person@example.com", "Question", "Body");
        let key = message.metadata.dedupe_key();
        let mail = FakeMail::new(vec![message]);
        let decisions = failing_safety_decisions();
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(mail.sent().is_empty());
        assert!(mail.seen().is_empty());
        assert_eq!(
            processing.status(&key),
            Some(PROCESSING_STATUS_RETRYABLE_FAILED.to_string())
        );
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "safety_scan" && event.status == "failed"));
    }

    #[tokio::test]
    async fn agent_failure_leaves_message_unseen() {
        let logger = crate::logging::MemoryLogger::default();
        let processing = MemoryProcessingStore::default();
        let message = inbound(46, "person@example.com", "Question", "Body");
        let key = message.metadata.dedupe_key();
        let mail = FakeMail::new(vec![message]);
        let decisions = failing_agent_decisions();
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(mail.sent().is_empty());
        assert!(mail.seen().is_empty());
        assert_eq!(
            processing.status(&key),
            Some(PROCESSING_STATUS_RETRYABLE_FAILED.to_string())
        );
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "agent_decision" && event.status == "failed"));
    }

    #[tokio::test]
    async fn send_failure_does_not_mark_seen() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(47, "person@example.com", "Question", "Body")])
            .with_fail_send();
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        assert!(mail.sent().is_empty());
        assert!(mail.seen().is_empty());
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "smtp_send" && event.status == "failed"));
    }

    #[tokio::test]
    async fn mark_seen_failure_is_logged_after_send() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(48, "person@example.com", "Question", "Body")])
            .with_fail_mark_seen();
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        assert_eq!(mail.sent().len(), 1);
        assert!(mail.seen().is_empty());
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "imap_mark_seen" && event.status == "failed"));
    }

    #[tokio::test]
    async fn sent_message_is_not_sent_again_when_mark_seen_failed() {
        let message = inbound(50, "person@example.com", "Question", "Body");
        let processing = MemoryProcessingStore::default();
        let first_logger = crate::logging::MemoryLogger::default();
        let first_mail = FakeMail::new(vec![message.clone()]).with_fail_mark_seen();
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with_store(
            &config(),
            &first_logger,
            "run-test",
            &first_mail,
            &decisions,
            &processing,
        )
        .await;

        assert_eq!(first_mail.sent().len(), 1);
        assert!(first_mail.seen().is_empty());
        assert_eq!(
            processing.status(&message.metadata.dedupe_key()),
            Some("replied".to_string())
        );

        let second_logger = crate::logging::MemoryLogger::default();
        let second_mail = FakeMail::new(vec![message]);
        run_once_with_store(
            &config(),
            &second_logger,
            "run-test-2",
            &second_mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(second_mail.sent().is_empty());
        assert_eq!(second_mail.seen()[0].uid, 50);
        assert!(second_logger
            .events()
            .iter()
            .any(|event| event.action == "processing_claim" && event.status == "dedupe_skip"));
    }

    #[tokio::test]
    async fn send_failure_can_be_retried_by_later_poll() {
        let message = inbound(51, "person@example.com", "Question", "Body");
        let processing = MemoryProcessingStore::default();
        let first_logger = crate::logging::MemoryLogger::default();
        let first_mail = FakeMail::new(vec![message.clone()]).with_fail_send();
        let decisions = fake_decisions(safe_scan(), reply_action());
        run_once_with_store(
            &config(),
            &first_logger,
            "run-test",
            &first_mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(first_mail.sent().is_empty());
        assert_eq!(
            processing.status(&message.metadata.dedupe_key()),
            Some(PROCESSING_STATUS_SEND_FAILED.to_string())
        );

        let second_logger = crate::logging::MemoryLogger::default();
        let second_mail = FakeMail::new(vec![message]);
        run_once_with_store(
            &config(),
            &second_logger,
            "run-test-2",
            &second_mail,
            &decisions,
            &processing,
        )
        .await;

        assert_eq!(second_mail.sent().len(), 1);
        assert_eq!(second_mail.seen()[0].uid, 51);
    }

    #[tokio::test]
    async fn in_progress_claim_defers_message_without_side_effects() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(52, "person@example.com", "Question", "Body")]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::InProgress]);
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(mail.sent().is_empty());
        assert!(mail.seen().is_empty());
        assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "processing_claim" && event.status == "in_progress"));
    }

    #[tokio::test]
    async fn claim_failure_logs_and_defers_message_without_side_effects() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(53, "person@example.com", "Question", "Body")]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::Fail]);
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(mail.sent().is_empty());
        assert!(mail.seen().is_empty());
        assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "processing_claim" && event.status == "failed"));
    }

    #[tokio::test]
    async fn already_finished_claim_marks_seen_without_sending() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(54, "person@example.com", "Question", "Body")]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        let processing = FakeProcessingStore::new(vec![FakeClaimOutcome::AlreadyFinished]);
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert!(mail.sent().is_empty());
        assert_eq!(mail.seen()[0].uid, 54);
        assert_eq!(decisions.call_counts(), DecisionCallCounts::default());
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "processing_claim" && event.status == "dedupe_skip"));
    }

    #[tokio::test]
    async fn processing_update_failure_is_logged_after_successful_send() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(55, "person@example.com", "Question", "Body")]);
        let decisions = fake_decisions(safe_scan(), reply_action());
        let processing =
            FakeProcessingStore::new(vec![FakeClaimOutcome::Claimed]).with_fail_update();
        run_once_with_store(
            &config(),
            &logger,
            "run-test",
            &mail,
            &decisions,
            &processing,
        )
        .await;

        assert_eq!(mail.sent().len(), 1);
        assert_eq!(mail.seen()[0].uid, 55);
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "processing_update" && event.status == "failed"));
    }

    #[tokio::test]
    async fn noop_marks_seen_without_sending() {
        let logger = crate::logging::MemoryLogger::default();
        let mail = FakeMail::new(vec![inbound(49, "person@example.com", "Question", "Body")]);
        let decisions = fake_decisions(safe_scan(), noop_action());
        run_once_with(&config(), &logger, "run-test", &mail, &decisions).await;

        assert!(mail.sent().is_empty());
        assert_eq!(mail.seen()[0].uid, 49);
        assert!(logger
            .events()
            .iter()
            .any(|event| event.action == "imap_mark_seen" && event.status == "noop"));
    }

    #[test]
    fn load_and_plan_reads_config_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        config().save(&path).unwrap();
        let plans = load_and_plan(&path).unwrap();
        assert_eq!(plans[0].mailbox_id, "support");
    }

    #[test]
    fn safety_disposition_controls_human_review_forward() {
        let decision = crate::safety::decide(&SafetyScanResult {
            category: SafetyCategory::Jailbreak,
            reason: "tries to override policy".to_string(),
            confidence: 0.95,
        });
        assert!(should_forward_for_human_review(&decision));
    }
}
