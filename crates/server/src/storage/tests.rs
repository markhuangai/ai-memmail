use super::*;
use crate::config::{
    AgentConfig, AiConfig, AiProtocol, DatabaseConfig, ImapConfig, LoggingConfig, MailboxConfig,
    PromptConfig, ReviewConfig, SmtpConfig,
};
use crate::mail::{InboundMessage, MessageMetadata};

fn message(uid: u64) -> InboundMessage {
    InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 1,
            uid,
            message_id: Some(format!("<{uid}@example.com>")),
            in_reply_to: None,
            references: vec![],
            from_addr: "person@example.com".to_string(),
            recipients: vec![],
            subject: "Question".to_string(),
        },
        plain_text: "Body".to_string(),
    }
}

fn app_config_with_mailboxes(ids: Vec<&str>) -> AppConfig {
    AppConfig {
        version: 1,
        database: DatabaseConfig {
            host: "postgres".to_string(),
            port: 5432,
            username: "user".to_string(),
            password: "secret".to_string(),
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
            email_classifier: "email-classifier.md".into(),
            rule_action: "rule-action.md".into(),
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
        mcp_servers: Default::default(),
        mailboxes: ids
            .into_iter()
            .map(|id| MailboxConfig {
                id: id.to_string(),
                address: format!("{id}@example.com"),
                enabled: true,
                poll_interval_seconds: 30,
                safety_forward_to: vec!["human@example.com".to_string()],
                accepted_conditions: vec![],
                mcp_servers: vec![],
                agent: AgentConfig {
                    system_prompt_path: "agent.md".into(),
                    default_forward_to: vec!["human@example.com".to_string()],
                },
                imap: ImapConfig {
                    host: "imap.example.com".to_string(),
                    port: 993,
                    tls: true,
                    username: format!("{id}@example.com"),
                    password: "secret".to_string(),
                    folder: "INBOX".to_string(),
                },
                smtp: SmtpConfig {
                    host: "smtp.example.com".to_string(),
                    port: 587,
                    starttls: true,
                    username: format!("{id}@example.com"),
                    password: "secret".to_string(),
                    from: format!("{id}@example.com"),
                },
            })
            .collect(),
        banned_senders: vec![],
    }
}

mod chunk_1;
mod chunk_2;
