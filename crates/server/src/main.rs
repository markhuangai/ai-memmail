use std::net::SocketAddr;
use std::path::PathBuf;

use ai_memmail_server::config::AppConfig;
use ai_memmail_server::logging::{init_tracing, LogLevel};
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

    let role = RuntimeRole::from_env_value(std::env::var("AI_MEMMAIL_ROLE").ok().as_deref())?;
    let bind = bind_from_env_value(std::env::var("AI_MEMMAIL_BIND").ok())?;

    match role {
        RuntimeRole::All => run_all(bind, config_path, config).await?,
        RuntimeRole::Web => web::serve(bind, config_path, config).await?,
        RuntimeRole::Worker => worker::run(config_path).await?,
    }

    Ok(())
}

async fn run_all(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeRole {
    All,
    Web,
    Worker,
}

impl RuntimeRole {
    fn from_env_value(value: Option<&str>) -> Result<Self, InvalidRuntimeRole> {
        match value.unwrap_or("all") {
            "" | "all" => Ok(Self::All),
            "web" => Ok(Self::Web),
            "worker" => Ok(Self::Worker),
            value => Err(InvalidRuntimeRole(value.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid AI_MEMMAIL_ROLE {0:?}; expected all, web, or worker")]
struct InvalidRuntimeRole(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_role_defaults_to_all() {
        assert_eq!(RuntimeRole::from_env_value(None).unwrap(), RuntimeRole::All);
        assert_eq!(
            RuntimeRole::from_env_value(Some("")).unwrap(),
            RuntimeRole::All
        );
    }

    #[test]
    fn runtime_role_accepts_explicit_modes() {
        assert_eq!(
            RuntimeRole::from_env_value(Some("all")).unwrap(),
            RuntimeRole::All
        );
        assert_eq!(
            RuntimeRole::from_env_value(Some("web")).unwrap(),
            RuntimeRole::Web
        );
        assert_eq!(
            RuntimeRole::from_env_value(Some("worker")).unwrap(),
            RuntimeRole::Worker
        );
    }

    #[test]
    fn runtime_role_rejects_unknown_modes() {
        let error = RuntimeRole::from_env_value(Some("api"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected all, web, or worker"));
    }

    #[test]
    fn config_path_defaults_to_config_yaml() {
        assert_eq!(
            config_path_from_env_value(None),
            PathBuf::from("config/config.yaml")
        );
    }
}
