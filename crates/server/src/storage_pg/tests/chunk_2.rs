fn app_config_with_mailboxes(mailbox_ids: Vec<&str>) -> AppConfig {
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
            email_classifier: "classifier.md".into(),
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
        mcp_servers: BTreeMap::new(),
        mailboxes: mailbox_ids
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
                    default_forward_to: vec![],
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

struct TestPgStore {
    store: PgStore,
    admin_config: DatabaseConfig,
    database_name: String,
}

impl TestPgStore {
    async fn create() -> Option<Self> {
        let admin_config = test_pg_admin_config()?;
        let database_name = format!("ai_memmail_test_{}", uuid::Uuid::new_v4().simple());
        let admin = connect_test_pg(&admin_config).await;
        admin
            .batch_execute(&format!("CREATE DATABASE {}", quote_ident(&database_name)))
            .await
            .unwrap();

        let mut store_config = admin_config.clone();
        store_config.database = database_name.clone();
        let store = PgStore::connect(&store_config).await.unwrap();
        Some(Self {
            store,
            admin_config,
            database_name,
        })
    }

    fn store_config(&self) -> DatabaseConfig {
        let mut config = self.admin_config.clone();
        config.database = self.database_name.clone();
        config
    }

    async fn cleanup(self) {
        let database_name = self.database_name;
        let admin_config = self.admin_config;
        drop(self.store);

        let admin = connect_test_pg(&admin_config).await;
        admin
            .execute(
                "SELECT pg_terminate_backend(pid)
                FROM pg_stat_activity
                WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&database_name],
            )
            .await
            .unwrap();
        admin
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {}",
                quote_ident(&database_name)
            ))
            .await
            .unwrap();
    }
}

fn test_pg_admin_config() -> Option<DatabaseConfig> {
    if std::env::var("AI_MEMMAIL_RUN_POSTGRES_TESTS")
        .ok()
        .as_deref()
        != Some("1")
    {
        return None;
    }
    let host = std::env::var("AI_MEMMAIL_TEST_PG_HOST").ok()?;
    let port = std::env::var("AI_MEMMAIL_TEST_PG_PORT")
        .ok()
        .and_then(|value| value.parse().ok())?;
    let username = std::env::var("AI_MEMMAIL_TEST_PG_USER").ok()?;
    let password = std::env::var("AI_MEMMAIL_TEST_PG_PASSWORD").ok()?;
    let database = std::env::var("AI_MEMMAIL_TEST_PG_DATABASE").ok()?;
    Some(DatabaseConfig {
        host,
        port,
        username,
        password,
        database,
    })
}

async fn connect_test_pg(config: &DatabaseConfig) -> tokio_postgres::Client {
    let mut postgres_config = tokio_postgres::Config::new();
    postgres_config
        .host(&config.host)
        .port(config.port)
        .user(&config.username)
        .password(&config.password)
        .dbname(&config.database);
    let (client, connection) = postgres_config
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
