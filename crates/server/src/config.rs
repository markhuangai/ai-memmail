use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const REDACTED_SECRET: &str = "********";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub version: u16,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub prompts: PromptConfig,
    pub ai: AiConfig,
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    pub mailboxes: Vec<MailboxConfig>,
    #[serde(default)]
    pub banned_senders: Vec<BannedSenderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub verbose_actions: bool,
    pub retention_days: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptConfig {
    pub root: PathBuf,
    pub safety_scan: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    Openai,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiConfig {
    pub protocol: AiProtocol,
    #[serde(rename = "AI_API_URL")]
    pub api_url: String,
    #[serde(rename = "AI_API_SECRET")]
    pub api_secret: String,
    #[serde(rename = "AI_MODEL")]
    pub model: String,
    pub review: ReviewConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewConfig {
    pub enabled: bool,
    pub prompt_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    StreamableHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub transport: McpTransport,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailboxConfig {
    pub id: String,
    pub address: String,
    pub enabled: bool,
    pub poll_interval_seconds: u64,
    pub safety_forward_to: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    pub agent: AgentConfig,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    pub system_prompt_path: PathBuf,
    #[serde(default)]
    pub default_forward_to: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub starttls: bool,
    pub username: String,
    pub password: String,
    pub from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BannedSenderKind {
    Email,
    Domain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BannedSenderConfig {
    pub kind: BannedSenderKind,
    pub value: String,
    pub reason: String,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        serde_yaml_ng::from_str(&content).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        let content = serde_yaml_ng::to_string(self).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        fs::write(path, content).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::Invalid("version must be 1".to_string()));
        }
        validate_required(&self.database.host, "database.host")?;
        if self.database.port == 0 {
            return Err(ConfigError::Invalid(
                "database.port must be greater than zero".to_string(),
            ));
        }
        validate_required(&self.database.username, "database.username")?;
        validate_required(&self.database.password, "database.password")?;
        validate_required(&self.database.database, "database.database")?;
        if !matches!(
            self.logging.level.as_str(),
            "debug" | "info" | "warn" | "error"
        ) {
            return Err(ConfigError::Invalid(
                "logging.level must be debug, info, warn, or error".to_string(),
            ));
        }
        if !matches!(self.logging.format.as_str(), "json" | "pretty") {
            return Err(ConfigError::Invalid(
                "logging.format must be json or pretty".to_string(),
            ));
        }
        if self.logging.retention_days == 0 {
            return Err(ConfigError::Invalid(
                "logging.retention_days must be greater than zero".to_string(),
            ));
        }
        validate_prompt_path(&self.prompts.safety_scan, "prompts.safety_scan")?;
        validate_prompt_path(&self.ai.review.prompt_path, "ai.review.prompt_path")?;

        let mut ids = BTreeSet::new();
        for mailbox in &self.mailboxes {
            validate_mailbox(mailbox, &self.mcp_servers, &mut ids)?;
        }
        Ok(())
    }

    pub fn redacted(&self) -> Self {
        let mut clone = self.clone();
        clone.database.password = redact(&clone.database.password);
        clone.ai.api_secret = redact(&clone.ai.api_secret);
        for server in clone.mcp_servers.values_mut() {
            for (key, value) in server.env.iter_mut() {
                if key.to_ascii_lowercase().contains("key")
                    || key.to_ascii_lowercase().contains("secret")
                    || key.to_ascii_lowercase().contains("token")
                {
                    *value = redact(value);
                }
            }
        }
        for mailbox in &mut clone.mailboxes {
            mailbox.imap.password = redact(&mailbox.imap.password);
            mailbox.smtp.password = redact(&mailbox.smtp.password);
        }
        clone
    }

    pub fn preserve_redacted_secrets(&mut self, current: &Self) {
        preserve_if_redacted(&mut self.database.password, &current.database.password);
        preserve_if_redacted(&mut self.ai.api_secret, &current.ai.api_secret);

        for (name, server) in &mut self.mcp_servers {
            if let Some(current_server) = current.mcp_servers.get(name) {
                for (key, value) in &mut server.env {
                    if let Some(current_value) = current_server.env.get(key) {
                        preserve_if_redacted(value, current_value);
                    }
                }
            }
        }

        for mailbox in &mut self.mailboxes {
            if let Some(current_mailbox) = current
                .mailboxes
                .iter()
                .find(|candidate| candidate.id == mailbox.id)
            {
                preserve_if_redacted(&mut mailbox.imap.password, &current_mailbox.imap.password);
                preserve_if_redacted(&mut mailbox.smtp.password, &current_mailbox.smtp.password);
            }
        }
    }
}

fn validate_required(value: &str, field: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::Invalid(format!("{field} is required")));
    }
    Ok(())
}

fn validate_mailbox(
    mailbox: &MailboxConfig,
    mcp_servers: &BTreeMap<String, McpServerConfig>,
    ids: &mut BTreeSet<String>,
) -> Result<(), ConfigError> {
    if mailbox.id.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "mailboxes[].id is required".to_string(),
        ));
    }
    if !ids.insert(mailbox.id.clone()) {
        return Err(ConfigError::Invalid(format!(
            "duplicate mailbox id {}",
            mailbox.id
        )));
    }
    if mailbox.poll_interval_seconds == 0 {
        return Err(ConfigError::Invalid(format!(
            "mailbox {} poll_interval_seconds must be greater than zero",
            mailbox.id
        )));
    }
    if mailbox.safety_forward_to.is_empty() {
        return Err(ConfigError::Invalid(format!(
            "mailbox {} safety_forward_to must not be empty",
            mailbox.id
        )));
    }
    validate_prompt_path(
        &mailbox.agent.system_prompt_path,
        "mailboxes[].agent.system_prompt_path",
    )?;
    for server in &mailbox.mcp_servers {
        if !mcp_servers.contains_key(server) {
            return Err(ConfigError::Invalid(format!(
                "mailbox {} references unknown MCP server {}",
                mailbox.id, server
            )));
        }
    }
    Ok(())
}

fn validate_prompt_path(path: &Path, field: &str) -> Result<(), ConfigError> {
    if path.as_os_str().is_empty() {
        return Err(ConfigError::Invalid(format!("{field} is required")));
    }
    if path.is_absolute() {
        return Err(ConfigError::Invalid(format!(
            "{field} must be relative to prompts.root"
        )));
    }
    if path.components().any(is_prompt_escape_component) {
        return Err(ConfigError::Invalid(format!(
            "{field} must not contain parent directory components"
        )));
    }
    Ok(())
}

fn is_prompt_escape_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::Prefix(_) | Component::RootDir
    )
}

fn redact(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        REDACTED_SECRET.to_string()
    }
}

fn preserve_if_redacted(next: &mut String, current: &str) {
    if next == REDACTED_SECRET {
        *next = current.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> AppConfig {
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
                root: "./prompts".into(),
                safety_scan: "safety-scan.md".into(),
            },
            ai: AiConfig {
                protocol: AiProtocol::Openai,
                api_url: "https://api.example/v1".to_string(),
                api_secret: "secret".to_string(),
                model: "model".to_string(),
                review: ReviewConfig {
                    enabled: false,
                    prompt_path: "outbound-review.md".into(),
                },
            },
            mcp_servers: BTreeMap::from([(
                "dense_mem".to_string(),
                McpServerConfig {
                    transport: McpTransport::Stdio,
                    command: Some("npx".to_string()),
                    args: vec!["-y".to_string(), "dense-mem-mcp-proxy".to_string()],
                    env: BTreeMap::from([("DENSE_MEM_API_KEY".to_string(), "dm".to_string())]),
                    url: None,
                },
            )]),
            mailboxes: vec![MailboxConfig {
                id: "support".to_string(),
                address: "support@example.com".to_string(),
                enabled: true,
                poll_interval_seconds: 60,
                safety_forward_to: vec!["human@example.com".to_string()],
                mcp_servers: vec!["dense_mem".to_string()],
                agent: AgentConfig {
                    system_prompt_path: "support-agent.md".into(),
                    default_forward_to: vec!["human@example.com".to_string()],
                },
                imap: ImapConfig {
                    host: "imap.example.com".to_string(),
                    port: 993,
                    tls: true,
                    username: "support@example.com".to_string(),
                    password: "imap-secret".to_string(),
                    folder: "INBOX".to_string(),
                },
                smtp: SmtpConfig {
                    host: "smtp.example.com".to_string(),
                    port: 587,
                    starttls: true,
                    username: "support@example.com".to_string(),
                    password: "smtp-secret".to_string(),
                    from: "support@example.com".to_string(),
                },
            }],
            banned_senders: vec![],
        }
    }

    #[test]
    fn validates_good_config() {
        assert!(valid_config().validate().is_ok());
    }

    #[test]
    fn rejects_duplicate_mailbox_ids() {
        let mut config = valid_config();
        config.mailboxes.push(config.mailboxes[0].clone());
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("duplicate mailbox id support"));
    }

    #[test]
    fn rejects_absolute_prompt_paths() {
        let mut config = valid_config();
        config.prompts.safety_scan = "/tmp/prompt.md".into();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("prompts.safety_scan must be relative"));
    }

    #[test]
    fn rejects_prompt_paths_with_parent_components() {
        let mut config = valid_config();
        config.prompts.safety_scan = "../config/local.yaml".into();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("prompts.safety_scan must not contain parent"));

        let mut config = valid_config();
        config.ai.review.prompt_path = "reviews/../secret.md".into();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("ai.review.prompt_path must not contain parent"));

        let mut config = valid_config();
        config.mailboxes[0].agent.system_prompt_path = "../agent.md".into();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("mailboxes[].agent.system_prompt_path must not contain parent"));
    }

    #[test]
    fn redacts_secrets_without_changing_shape() {
        let redacted = valid_config().redacted();
        assert_eq!(redacted.database.password, "********");
        assert_eq!(redacted.ai.api_secret, "********");
        assert_eq!(
            redacted.mcp_servers["dense_mem"].env["DENSE_MEM_API_KEY"],
            "********"
        );
        assert_eq!(redacted.mailboxes[0].imap.password, "********");
        assert_eq!(redacted.mailboxes[0].smtp.password, "********");
    }

    #[test]
    fn preserves_redacted_secrets_before_saving() {
        let current = valid_config();
        let mut next = current.redacted();
        next.database.host = "db.changed.test".to_string();
        next.mcp_servers.get_mut("dense_mem").unwrap().env.insert(
            "DENSE_MEM_MCP_URL".to_string(),
            "http://changed.test".to_string(),
        );
        next.preserve_redacted_secrets(&current);

        assert_eq!(next.database.password, "db-secret");
        assert_eq!(next.ai.api_secret, "secret");
        assert_eq!(next.mcp_servers["dense_mem"].env["DENSE_MEM_API_KEY"], "dm");
        assert_eq!(
            next.mcp_servers["dense_mem"].env["DENSE_MEM_MCP_URL"],
            "http://changed.test"
        );
        assert_eq!(next.mailboxes[0].imap.password, "imap-secret");
        assert_eq!(next.mailboxes[0].smtp.password, "smtp-secret");
        assert_eq!(next.database.host, "db.changed.test");
    }

    #[test]
    fn rejects_unknown_log_format() {
        let mut config = valid_config();
        config.logging.format = "xml".to_string();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("logging.format must be json or pretty"));
    }

    #[test]
    fn rejects_invalid_top_level_settings() {
        let mut config = valid_config();
        config.version = 2;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("version"));

        let mut config = valid_config();
        config.database.host.clear();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("database.host"));

        let mut config = valid_config();
        config.database.port = 0;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("database.port"));

        let mut config = valid_config();
        config.database.username.clear();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("database.username"));

        let mut config = valid_config();
        config.database.password.clear();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("database.password"));

        let mut config = valid_config();
        config.database.database.clear();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("database.database"));

        let mut config = valid_config();
        config.logging.level = "trace".to_string();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("logging.level"));

        let mut config = valid_config();
        config.logging.retention_days = 0;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("retention_days"));
    }

    #[test]
    fn rejects_invalid_mailbox_settings() {
        let mut config = valid_config();
        config.mailboxes[0].id.clear();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("mailboxes[].id"));

        let mut config = valid_config();
        config.mailboxes[0].poll_interval_seconds = 0;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("poll_interval_seconds"));

        let mut config = valid_config();
        config.mailboxes[0].safety_forward_to.clear();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("safety_forward_to"));

        let mut config = valid_config();
        config.mailboxes[0].mcp_servers = vec!["missing".to_string()];
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unknown MCP server"));
    }

    #[test]
    fn rejects_empty_prompt_paths() {
        let mut config = valid_config();
        config.prompts.safety_scan = PathBuf::new();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("required"));

        let mut config = valid_config();
        config.mailboxes[0].agent.system_prompt_path = PathBuf::new();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("required"));
    }

    #[test]
    fn saves_valid_yaml_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let config = valid_config();
        config.save(&path).unwrap();
        let loaded = AppConfig::load(&path).unwrap();
        assert_eq!(loaded, config);
    }

    #[test]
    fn load_reports_read_and_parse_errors() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.yaml");
        let error = AppConfig::load(&missing).unwrap_err().to_string();
        assert!(error.contains("failed to read config"));

        let invalid = dir.path().join("invalid.yaml");
        std::fs::write(&invalid, "version: [").unwrap();
        let error = AppConfig::load(&invalid).unwrap_err().to_string();
        assert!(error.contains("failed to parse config"));
    }
}
