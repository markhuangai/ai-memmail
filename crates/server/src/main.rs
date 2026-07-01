use std::net::SocketAddr;
use std::path::PathBuf;

use ai_memmail_server::config::AppConfig;
use ai_memmail_server::logging::{init_tracing, LogLevel};
use ai_memmail_server::storage::PgStore;
use ai_memmail_server::{web, worker};

const DEFAULT_CONFIG_PATH: &str = "config/config.yaml";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("level=fatal action=startup status=failed error={error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = config_path_from_env_value(std::env::var("AI_MEMMAIL_CONFIG").ok());
    let config = AppConfig::load(&config_path)?;
    config.validate()?;

    init_tracing(LogLevel::from(config.logging.level.as_str()));

    let migration_store = PgStore::connect(&config.database).await?;
    migration_store.migrate().await?;
    drop(migration_store);

    let bind = bind_from_env_value(std::env::var("AI_MEMMAIL_BIND").ok())?;
    run_services(bind, config_path, config).await?;

    Ok(())
}

async fn run_services(
    bind: SocketAddr,
    config_path: PathBuf,
    config: AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::select! {
        result = web::serve(bind, config_path.clone(), config) => result?,
        result = worker::run(config_path) => result?,
    }
    Ok(())
}

fn config_path_from_env_value(value: Option<String>) -> PathBuf {
    value
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
}

fn bind_from_env_value(value: Option<String>) -> Result<SocketAddr, std::net::AddrParseError> {
    value
        .unwrap_or_else(|| "0.0.0.0:8080".to_string())
        .parse::<SocketAddr>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_defaults_to_config_yaml() {
        assert_eq!(
            config_path_from_env_value(None),
            PathBuf::from("config/config.yaml")
        );
    }
}
