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
            accepted_conditions: vec![],
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

fn assert_invalid_config(config: AppConfig, expected: &str) {
    let error = config.validate().unwrap_err().to_string();
    assert!(
        error.contains(expected),
        "expected {error:?} to contain {expected:?}"
    );
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
    config.prompts.safety_scan = "../config/config.yaml".into();
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("prompts.safety_scan must not contain parent"));

    let mut config = valid_config();
    config.prompts.email_classifier = "../classifier.md".into();
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("prompts.email_classifier must not contain parent"));

    let mut config = valid_config();
    config.prompts.rule_action = "../rule-action.md".into();
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("prompts.rule_action must not contain parent"));

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
    let mut config = valid_config();
    config.database.password.clear();
    config
        .mcp_servers
        .get_mut("dense_mem")
        .unwrap()
        .env
        .insert("EMPTY_SECRET".to_string(), String::new());

    let redacted = config.redacted();
    assert_eq!(redacted.database.password, "");
    assert_eq!(redacted.mcp_servers["dense_mem"].env["EMPTY_SECRET"], "");

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
fn redacts_sensitive_mcp_env_names() {
    let mut config = valid_config();
    let server = config.mcp_servers.get_mut("dense_mem").unwrap();
    server
        .env
        .insert("DENSE_MEM_PASSWORD".to_string(), "password".to_string());
    server
        .env
        .insert("DENSE_MEM_PASS".to_string(), "pass".to_string());
    server
        .env
        .insert("DENSE_MEM_PWD".to_string(), "pwd".to_string());
    server
        .env
        .insert("DENSE_MEM_AUTH".to_string(), "auth".to_string());
    server
        .env
        .insert("DENSE_MEM_CREDENTIAL".to_string(), "credential".to_string());
    server.env.insert(
        "DENSE_MEM_PRIVATE_KEY".to_string(),
        "private-key".to_string(),
    );
    server
        .env
        .insert("HTTP_AUTHORIZATION".to_string(), "bearer-token".to_string());
    server
        .env
        .insert("DENSE_MEM_APIKEY".to_string(), "api-key".to_string());
    server.env.insert(
        "DENSE_MEM_MCP_URL".to_string(),
        "http://dense-mem".to_string(),
    );

    let redacted = config.redacted();
    let env = &redacted.mcp_servers["dense_mem"].env;
    for key in [
        "DENSE_MEM_API_KEY",
        "DENSE_MEM_PASSWORD",
        "DENSE_MEM_PASS",
        "DENSE_MEM_PWD",
        "DENSE_MEM_AUTH",
        "DENSE_MEM_CREDENTIAL",
        "DENSE_MEM_PRIVATE_KEY",
        "HTTP_AUTHORIZATION",
        "DENSE_MEM_APIKEY",
    ] {
        assert_eq!(env[key], "********");
    }
    assert_eq!(env["DENSE_MEM_MCP_URL"], "http://dense-mem");
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
fn preserves_redacted_mcp_secrets_when_server_is_renamed() {
    let current = valid_config();
    let mut next = current.redacted();
    let renamed = next.mcp_servers.remove("dense_mem").unwrap();
    next.mcp_servers
        .insert("project_memory".to_string(), renamed);
    next.mailboxes[0].mcp_servers = vec!["project_memory".to_string()];

    next.preserve_redacted_secrets(&current);

    assert_eq!(
        next.mcp_servers["project_memory"].env["DENSE_MEM_API_KEY"],
        "dm"
    );
}

#[test]
fn preserves_redacted_mcp_secrets_when_renamed_server_is_edited() {
    let current = valid_config();
    let mut next = current.redacted();
    let mut renamed = next.mcp_servers.remove("dense_mem").unwrap();
    renamed.command = Some("node".to_string());
    renamed.args = vec!["server.js".to_string()];
    renamed.env.insert(
        "DENSE_MEM_MCP_URL".to_string(),
        "http://changed.test".to_string(),
    );
    next.mcp_servers
        .insert("project_memory".to_string(), renamed);
    next.mailboxes[0].mcp_servers = vec!["project_memory".to_string()];

    next.preserve_redacted_secrets(&current);

    assert_eq!(
        next.mcp_servers["project_memory"].env["DENSE_MEM_API_KEY"],
        "dm"
    );
    assert_eq!(
        next.mcp_servers["project_memory"].env["DENSE_MEM_MCP_URL"],
        "http://changed.test"
    );
}

#[test]
fn does_not_preserve_redacted_mcp_rename_when_match_is_ambiguous() {
    let mut current = valid_config();
    let mut second = current.mcp_servers["dense_mem"].clone();
    second
        .env
        .insert("DENSE_MEM_API_KEY".to_string(), "dm-secondary".to_string());
    current
        .mcp_servers
        .insert("dense_mem_secondary".to_string(), second);

    let mut next = current.redacted();
    let renamed = next.mcp_servers.remove("dense_mem").unwrap();
    next.mcp_servers.remove("dense_mem_secondary");
    next.mcp_servers
        .insert("project_memory".to_string(), renamed);
    next.mailboxes[0].mcp_servers = vec!["project_memory".to_string()];

    next.preserve_redacted_secrets(&current);

    assert_eq!(
        next.mcp_servers["project_memory"].env["DENSE_MEM_API_KEY"],
        "********"
    );
}

#[test]
fn prompt_config_defaults_classifier_and_rule_paths() {
    let prompts: PromptConfig = serde_yaml_ng::from_str(
        r#"
root: prompts
safety_scan: safety.md
"#,
    )
    .unwrap();

    assert_eq!(
        prompts.email_classifier,
        PathBuf::from("email-classifier.md")
    );
    assert_eq!(prompts.rule_action, PathBuf::from("rule-action.md"));
}

#[test]
fn mailbox_config_defaults_accepted_conditions() {
    let mailbox: MailboxConfig = serde_yaml_ng::from_str(
        r#"
id: support
address: support@example.com
enabled: false
poll_interval_seconds: 60
safety_forward_to: ["human@example.com"]
agent:
  system_prompt_path: support-agent.md
imap:
  host: imap.example.com
  port: 993
  tls: true
  username: support@example.com
  password: secret
  folder: INBOX
smtp:
  host: smtp.example.com
  port: 587
  starttls: true
  username: support@example.com
  password: secret
  from: support@example.com
"#,
    )
    .unwrap();

    assert!(mailbox.accepted_conditions.is_empty());
}

#[test]
fn rejects_empty_accepted_condition_groups() {
    let mut config = valid_config();
    config.mailboxes[0]
        .accepted_conditions
        .push(AcceptedCondition::default());

    assert_invalid_config(config, "accepted_conditions[0] must define");
}

#[test]
fn rejects_invalid_accepted_condition_regex() {
    let mut config = valid_config();
    config.mailboxes[0]
        .accepted_conditions
        .push(AcceptedCondition {
            recipients: vec![],
            subject_regex: vec!["(".to_string()],
        });

    assert_invalid_config(config, "accepted_conditions[0].subject_regex[0] is invalid");
}

#[test]
fn rejects_invalid_accepted_condition_recipients() {
    let mut config = valid_config();
    config.mailboxes[0]
        .accepted_conditions
        .push(AcceptedCondition {
            recipients: vec!["not-an-address".to_string()],
            subject_regex: vec![],
        });

    assert_invalid_config(
        config,
        "accepted_conditions[0].recipients[0] must be an email address",
    );
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
    assert_invalid_config(config, "mailboxes[].id");

    let mut config = valid_config();
    config.mailboxes[0].poll_interval_seconds = 0;
    assert_invalid_config(config, "poll_interval_seconds");

    let mut config = valid_config();
    config.mailboxes[0].safety_forward_to.clear();
    assert_invalid_config(config, "safety_forward_to");

    let mut config = valid_config();
    config.mailboxes[0].mcp_servers = vec!["missing".to_string()];
    assert_invalid_config(config, "unknown MCP server");
}

#[test]
fn rejects_incomplete_enabled_mailbox_connection_settings() {
    let mut config = valid_config();
    config.mailboxes[0].address.clear();
    assert_invalid_config(config, "mailbox support address is required");

    let mut config = valid_config();
    config.mailboxes[0].imap.host.clear();
    assert_invalid_config(config, "mailbox support imap.host is required");

    let mut config = valid_config();
    config.mailboxes[0].imap.port = 0;
    assert_invalid_config(
        config,
        "mailbox support imap.port must be greater than zero",
    );

    let mut config = valid_config();
    config.mailboxes[0].imap.username.clear();
    assert_invalid_config(config, "mailbox support imap.username is required");

    let mut config = valid_config();
    config.mailboxes[0].imap.password.clear();
    assert_invalid_config(config, "mailbox support imap.password is required");

    let mut config = valid_config();
    config.mailboxes[0].imap.folder.clear();
    assert_invalid_config(config, "mailbox support imap.folder is required");

    let mut config = valid_config();
    config.mailboxes[0].smtp.host.clear();
    assert_invalid_config(config, "mailbox support smtp.host is required");

    let mut config = valid_config();
    config.mailboxes[0].smtp.port = 0;
    assert_invalid_config(
        config,
        "mailbox support smtp.port must be greater than zero",
    );

    let mut config = valid_config();
    config.mailboxes[0].smtp.username.clear();
    assert_invalid_config(config, "mailbox support smtp.username is required");

    let mut config = valid_config();
    config.mailboxes[0].smtp.password.clear();
    assert_invalid_config(config, "mailbox support smtp.password is required");

    let mut config = valid_config();
    config.mailboxes[0].smtp.from.clear();
    assert_invalid_config(config, "mailbox support smtp.from is required");
}

#[test]
fn allows_disabled_mailbox_drafts_without_connection_settings() {
    let mut config = valid_config();
    let mailbox = &mut config.mailboxes[0];
    mailbox.enabled = false;
    mailbox.address.clear();
    mailbox.imap.host.clear();
    mailbox.imap.port = 0;
    mailbox.imap.username.clear();
    mailbox.imap.password.clear();
    mailbox.imap.folder.clear();
    mailbox.smtp.host.clear();
    mailbox.smtp.port = 0;
    mailbox.smtp.username.clear();
    mailbox.smtp.password.clear();
    mailbox.smtp.from.clear();

    assert!(config.validate().is_ok());
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
    config.prompts.email_classifier = PathBuf::new();
    assert!(config
        .validate()
        .unwrap_err()
        .to_string()
        .contains("required"));

    let mut config = valid_config();
    config.prompts.rule_action = PathBuf::new();
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
