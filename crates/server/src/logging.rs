use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl From<&str> for LogLevel {
    fn from(value: &str) -> Self {
        match value {
            "debug" => Self::Debug,
            "warn" => Self::Warn,
            "error" => Self::Error,
            "fatal" => Self::Fatal,
            _ => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionEvent {
    pub level: LogLevel,
    pub run_id: String,
    pub mailbox_id: Option<String>,
    pub message_uid_validity: Option<u64>,
    pub message_uid: Option<u64>,
    pub action: String,
    pub status: String,
    pub duration_ms: u128,
    pub detail: Option<String>,
}

#[async_trait]
pub trait ActionLogger: Send + Sync {
    async fn log(&self, event: ActionEvent);
}

#[derive(Debug, Default, Clone)]
pub struct MemoryLogger {
    events: Arc<Mutex<Vec<ActionEvent>>>,
}

impl MemoryLogger {
    pub fn events(&self) -> Vec<ActionEvent> {
        self.events.lock().expect("memory logger poisoned").clone()
    }
}

#[async_trait]
impl ActionLogger for MemoryLogger {
    async fn log(&self, event: ActionEvent) {
        self.events
            .lock()
            .expect("memory logger poisoned")
            .push(event);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StdoutLogger;

#[async_trait]
impl ActionLogger for StdoutLogger {
    async fn log(&self, event: ActionEvent) {
        match event.level {
            LogLevel::Debug => tracing::debug!(?event),
            LogLevel::Info => tracing::info!(?event),
            LogLevel::Warn => tracing::warn!(?event),
            LogLevel::Error | LogLevel::Fatal => tracing::error!(?event),
        }
    }
}

pub struct FanoutLogger<'a> {
    first: &'a dyn ActionLogger,
    second: &'a dyn ActionLogger,
}

impl<'a> FanoutLogger<'a> {
    pub fn new(first: &'a dyn ActionLogger, second: &'a dyn ActionLogger) -> Self {
        Self { first, second }
    }
}

#[async_trait]
impl ActionLogger for FanoutLogger<'_> {
    async fn log(&self, event: ActionEvent) {
        self.first.log(event.clone()).await;
        self.second.log(event).await;
    }
}

pub fn action_event(
    level: LogLevel,
    run_id: impl Into<String>,
    action: impl Into<String>,
    status: impl Into<String>,
    duration: Duration,
) -> ActionEvent {
    ActionEvent {
        level,
        run_id: run_id.into(),
        mailbox_id: None,
        message_uid_validity: None,
        message_uid: None,
        action: action.into(),
        status: status.into(),
        duration_ms: duration.as_millis(),
        detail: None,
    }
}

pub fn init_tracing(level: LogLevel) {
    let filter = EnvFilter::new(match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error | LogLevel::Fatal => "error",
    });
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_logger_records_action_events() {
        let logger = MemoryLogger::default();
        logger
            .log(action_event(
                LogLevel::Info,
                "run-1",
                "poll",
                "ok",
                Duration::from_millis(12),
            ))
            .await;
        let events = logger.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].duration_ms, 12);
    }

    #[tokio::test]
    async fn stdout_logger_accepts_all_levels() {
        init_tracing(LogLevel::Debug);
        let logger = StdoutLogger;
        for level in [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Fatal,
        ] {
            logger
                .log(action_event(
                    level,
                    "run-stdout",
                    "test",
                    "ok",
                    Duration::from_millis(1),
                ))
                .await;
        }
    }

    #[tokio::test]
    async fn fanout_logger_writes_to_both_loggers() {
        let first = MemoryLogger::default();
        let second = MemoryLogger::default();
        let logger = FanoutLogger::new(&first, &second);
        logger
            .log(action_event(
                LogLevel::Info,
                "run-fanout",
                "test",
                "ok",
                Duration::from_millis(3),
            ))
            .await;

        assert_eq!(first.events().len(), 1);
        assert_eq!(second.events().len(), 1);
        assert_eq!(first.events()[0], second.events()[0]);
    }

    #[test]
    fn action_event_populates_required_fields() {
        let event = action_event(
            LogLevel::Warn,
            "run-2",
            "scan",
            "quarantined",
            Duration::from_millis(42),
        );
        assert_eq!(event.level, LogLevel::Warn);
        assert_eq!(event.run_id, "run-2");
        assert_eq!(event.action, "scan");
        assert_eq!(event.status, "quarantined");
        assert_eq!(event.duration_ms, 42);
        assert_eq!(event.mailbox_id, None);
    }

    #[test]
    fn parses_fatal_as_application_log_level() {
        assert_eq!(LogLevel::from("fatal"), LogLevel::Fatal);
        assert_eq!(LogLevel::from("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::from("warn"), LogLevel::Warn);
        assert_eq!(LogLevel::from("error"), LogLevel::Error);
        assert_eq!(LogLevel::from("unknown"), LogLevel::Info);
    }
}
