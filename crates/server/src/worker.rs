use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::config::{AppConfig, ConfigError, MailboxConfig};
use crate::logging::{action_event, ActionLogger, LogLevel, StdoutLogger};
use crate::safety::{sender_is_banned, SafetyDecision, SafetyDisposition};

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

pub async fn run(config_path: PathBuf) -> Result<(), ConfigError> {
    let logger = StdoutLogger;
    loop {
        let started = Instant::now();
        let config = AppConfig::load(&config_path)?;
        config.validate()?;
        let run_id = Uuid::new_v4().to_string();
        run_once(&config, &logger, &run_id).await;
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{
        AgentConfig, AiConfig, AiProtocol, BannedSenderConfig, BannedSenderKind, DatabaseConfig,
        ImapConfig, LoggingConfig, PromptConfig, ReviewConfig, SmtpConfig,
    };
    use crate::safety::{SafetyCategory, SafetyScanResult};

    use super::*;

    fn config() -> AppConfig {
        AppConfig {
            version: 1,
            database: DatabaseConfig {
                url: "postgres://user:pass@localhost/db".to_string(),
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
        run_once(&config(), &logger, "run-test").await;
        let events = logger.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "worker_poll_plan");
        assert_eq!(events[0].status, "mailboxes=1");
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
