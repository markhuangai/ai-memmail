use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

mod signature;

use crate::html_sanitizer::sanitize_email_html;
use signature::validate_mailbox_signature;

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
    #[serde(default = "default_email_classifier_prompt")]
    pub email_classifier: PathBuf,
    #[serde(default = "default_rule_action_prompt")]
    pub rule_action: PathBuf,
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
    pub signature: Option<EmailSignatureConfig>,
    #[serde(default)]
    pub accepted_conditions: Vec<AcceptedCondition>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    pub agent: AgentConfig,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailSignatureConfig {
    pub format: EmailSignatureFormat,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailSignatureFormat {
    PlainText,
    Html,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedCondition {
    #[serde(default)]
    pub recipients: Vec<String>,
    #[serde(default)]
    pub subject_regex: Vec<String>,
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
    #[serde(default)]
    pub sent_folder: Option<String>,
    #[serde(default = "default_sent_backfill_days")]
    pub sent_backfill_days: u16,
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
        let config = self.sanitized_for_save();
        config.validate()?;
        let content = serde_yaml_ng::to_string(&config).map_err(|source| ConfigError::Parse {
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
        validate_prompt_path(&self.prompts.email_classifier, "prompts.email_classifier")?;
        validate_prompt_path(&self.prompts.rule_action, "prompts.rule_action")?;
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
                if is_sensitive_env_name(key) {
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

    pub fn sanitized_for_save(&self) -> Self {
        let mut clone = self.clone();
        for mailbox in &mut clone.mailboxes {
            let Some(signature) = &mut mailbox.signature else {
                continue;
            };
            if matches!(signature.format, EmailSignatureFormat::Html) {
                signature.content = sanitize_email_html(&signature.content).html;
            }
        }
        clone
    }

    pub fn preserve_redacted_secrets(&mut self, current: &Self) {
        preserve_if_redacted(&mut self.database.password, &current.database.password);
        preserve_if_redacted(&mut self.ai.api_secret, &current.ai.api_secret);

        let renamed_mcp_servers = renamed_mcp_server_matches(self, current);
        for (name, server) in &mut self.mcp_servers {
            let current_server = current.mcp_servers.get(name).or_else(|| {
                renamed_mcp_servers
                    .get(name)
                    .and_then(|current_name| current.mcp_servers.get(current_name))
            });
            if let Some(current_server) = current_server {
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

fn renamed_mcp_server_matches(next: &AppConfig, current: &AppConfig) -> BTreeMap<String, String> {
    let added_names = next
        .mcp_servers
        .keys()
        .filter(|name| !current.mcp_servers.contains_key(*name))
        .collect::<Vec<_>>();
    let removed_names = current
        .mcp_servers
        .keys()
        .filter(|name| !next.mcp_servers.contains_key(*name))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();

    for next_name in added_names {
        let next_server = &next.mcp_servers[next_name];
        for current_name in &removed_names {
            let current_server = &current.mcp_servers[*current_name];
            if mcp_server_matches_for_secret_preservation(next_server, current_server) {
                candidates.push((next_name, *current_name));
            }
        }
    }

    let mut matches = BTreeMap::new();
    for (next_name, current_name) in &candidates {
        let next_match_count = candidates
            .iter()
            .filter(|(candidate_next, _)| candidate_next == next_name)
            .count();
        let current_match_count = candidates
            .iter()
            .filter(|(_, candidate_current)| candidate_current == current_name)
            .count();
        if next_match_count == 1 && current_match_count == 1 {
            matches.insert((*next_name).clone(), (*current_name).clone());
        }
    }
    matches
}

fn mcp_server_matches_for_secret_preservation(
    next: &McpServerConfig,
    current: &McpServerConfig,
) -> bool {
    let redacted_keys = next
        .env
        .iter()
        .filter(|(key, value)| is_sensitive_env_name(key) && value.as_str() == REDACTED_SECRET)
        .map(|(key, _)| key)
        .collect::<Vec<_>>();

    !redacted_keys.is_empty()
        && redacted_keys
            .iter()
            .all(|key| current.env.contains_key(*key))
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
    validate_mailbox_signature(mailbox)?;
    validate_accepted_conditions(mailbox)?;
    if mailbox.enabled {
        validate_enabled_mailbox_connection(mailbox)?;
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

fn validate_accepted_conditions(mailbox: &MailboxConfig) -> Result<(), ConfigError> {
    for (index, condition) in mailbox.accepted_conditions.iter().enumerate() {
        let field = format!("mailbox {} accepted_conditions[{index}]", mailbox.id);
        let has_recipients = condition
            .recipients
            .iter()
            .any(|recipient| !recipient.trim().is_empty());
        let has_subject_regex = condition
            .subject_regex
            .iter()
            .any(|pattern| !pattern.trim().is_empty());
        if !has_recipients && !has_subject_regex {
            return Err(ConfigError::Invalid(format!(
                "{field} must define recipients or subject_regex"
            )));
        }
        for (recipient_index, recipient) in condition.recipients.iter().enumerate() {
            if recipient.trim().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "{field}.recipients[{recipient_index}] must not be empty"
                )));
            }
            if !recipient.trim().contains('@') {
                return Err(ConfigError::Invalid(format!(
                    "{field}.recipients[{recipient_index}] must be an email address"
                )));
            }
        }
        for (pattern_index, pattern) in condition.subject_regex.iter().enumerate() {
            if pattern.trim().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "{field}.subject_regex[{pattern_index}] must not be empty"
                )));
            }
            Regex::new(pattern).map_err(|error| {
                ConfigError::Invalid(format!(
                    "{field}.subject_regex[{pattern_index}] is invalid: {error}"
                ))
            })?;
        }
    }
    Ok(())
}

fn validate_enabled_mailbox_connection(mailbox: &MailboxConfig) -> Result<(), ConfigError> {
    let prefix = format!("mailbox {}", mailbox.id);
    validate_required(&mailbox.address, &format!("{prefix} address"))?;
    validate_required(&mailbox.imap.host, &format!("{prefix} imap.host"))?;
    validate_port(mailbox.imap.port, &format!("{prefix} imap.port"))?;
    validate_required(&mailbox.imap.username, &format!("{prefix} imap.username"))?;
    validate_required(&mailbox.imap.password, &format!("{prefix} imap.password"))?;
    validate_required(&mailbox.imap.folder, &format!("{prefix} imap.folder"))?;
    if mailbox
        .imap
        .sent_folder
        .as_ref()
        .is_some_and(|folder| folder.trim().is_empty())
    {
        return Err(ConfigError::Invalid(format!(
            "{prefix} imap.sent_folder must not be empty when provided"
        )));
    }
    validate_required(&mailbox.smtp.host, &format!("{prefix} smtp.host"))?;
    validate_port(mailbox.smtp.port, &format!("{prefix} smtp.port"))?;
    validate_required(&mailbox.smtp.username, &format!("{prefix} smtp.username"))?;
    validate_required(&mailbox.smtp.password, &format!("{prefix} smtp.password"))?;
    validate_required(&mailbox.smtp.from, &format!("{prefix} smtp.from"))?;
    Ok(())
}

fn validate_port(port: u16, field: &str) -> Result<(), ConfigError> {
    if port == 0 {
        return Err(ConfigError::Invalid(format!(
            "{field} must be greater than zero"
        )));
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

fn default_email_classifier_prompt() -> PathBuf {
    "email-classifier.md".into()
}

fn default_rule_action_prompt() -> PathBuf {
    "rule-action.md".into()
}

fn default_sent_backfill_days() -> u16 {
    30
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

fn is_sensitive_env_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("key")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("password")
        || lower.contains("passwd")
        || lower.contains("credential")
        || lower.contains("private_key")
        || lower.contains("private-key")
        || lower.contains("authorization")
        || lower
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|part| matches!(part, "pass" | "pwd" | "auth"))
}

fn preserve_if_redacted(next: &mut String, current: &str) {
    if next == REDACTED_SECRET {
        *next = current.to_string();
    }
}

#[cfg(test)]
mod tests;
