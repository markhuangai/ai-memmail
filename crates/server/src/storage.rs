use crate::config::DatabaseConfig;
use crate::logging::{ActionEvent, ActionLogger, LogLevel};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("postgres connection failed: {0}")]
    Connect(#[from] tokio_postgres::Error),
}

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");

#[derive(Debug)]
pub struct PgStore {
    client: tokio_postgres::Client,
}

impl PgStore {
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, StorageError> {
        let mut postgres_config = tokio_postgres::Config::new();
        postgres_config
            .host(&config.host)
            .port(config.port)
            .user(&config.username)
            .password(&config.password)
            .dbname(&config.database);
        let (client, connection) = postgres_config.connect(tokio_postgres::NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::error!(%error, "postgres connection task failed");
            }
        });
        Ok(Self { client })
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        self.client.batch_execute(INIT_SQL).await?;
        Ok(())
    }

    pub async fn insert_action_log(&self, event: &ActionEvent) -> Result<(), StorageError> {
        let message_uid = event.message_uid.map(|uid| uid as i64);
        self.client
            .execute(
                "INSERT INTO action_logs
                (level, run_id, mailbox_id, message_uid, action, status, duration_ms, detail)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &log_level_value(event.level),
                    &event.run_id,
                    &event.mailbox_id,
                    &message_uid,
                    &event.action,
                    &event.status,
                    &(event.duration_ms as i64),
                    &event.detail,
                ],
            )
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ActionLogger for PgStore {
    async fn log(&self, event: ActionEvent) {
        if let Err(error) = self.insert_action_log(&event).await {
            tracing::error!(%error, ?event, "failed to persist action log");
        }
    }
}

pub fn metadata_only_schema_guard(sql: &str) -> Result<(), String> {
    let lowered = sql.to_ascii_lowercase();
    let forbidden = [
        " raw_email",
        "email_body",
        " body text",
        " body bytea",
        " parsed_content",
        " message_content",
    ];
    for token in forbidden {
        if lowered.contains(token) {
            return Err(format!(
                "migration contains forbidden email-content column token: {token}"
            ));
        }
    }
    Ok(())
}

pub fn retention_delete_sql(retention_days: u16) -> String {
    format!(
        "DELETE FROM action_logs WHERE created_at < now() - interval '{} days'",
        retention_days
    )
}

fn log_level_value(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
        LogLevel::Fatal => "fatal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_metadata_only() {
        metadata_only_schema_guard(INIT_SQL).unwrap();
    }

    #[test]
    fn metadata_guard_rejects_content_columns() {
        let error = metadata_only_schema_guard("CREATE TABLE t (email_body TEXT)")
            .unwrap_err()
            .to_string();
        assert!(error.contains("forbidden email-content"));
    }

    #[test]
    fn migration_defines_expected_tables() {
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS processing_runs"));
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS action_logs"));
        assert!(INIT_SQL.contains("CREATE TABLE IF NOT EXISTS banned_senders"));
    }

    #[test]
    fn retention_sql_uses_configured_days() {
        assert_eq!(
            retention_delete_sql(180),
            "DELETE FROM action_logs WHERE created_at < now() - interval '180 days'"
        );
    }

    #[test]
    fn level_values_match_storage_check_constraint() {
        assert_eq!(log_level_value(LogLevel::Fatal), "fatal");
        assert_eq!(log_level_value(LogLevel::Debug), "debug");
        assert_eq!(log_level_value(LogLevel::Info), "info");
        assert_eq!(log_level_value(LogLevel::Warn), "warn");
        assert_eq!(log_level_value(LogLevel::Error), "error");
    }
}
