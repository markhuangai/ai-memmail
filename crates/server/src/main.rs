use std::net::SocketAddr;
use std::path::PathBuf;

use ai_memmail_server::config::AppConfig;
use ai_memmail_server::logging::{init_tracing, LogLevel};
use ai_memmail_server::{web, worker};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("level=fatal action=startup status=failed error={error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("AI_MEMMAIL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config/local.yaml"));
    let config = AppConfig::load(&config_path)?;
    config.validate()?;

    init_tracing(LogLevel::from(config.logging.level.as_str()));

    match std::env::var("AI_MEMMAIL_ROLE").as_deref() {
        Ok("worker") => worker::run(config_path).await?,
        _ => {
            let bind = std::env::var("AI_MEMMAIL_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
                .parse::<SocketAddr>()?;
            web::serve(bind, config_path, config).await?;
        }
    }

    Ok(())
}
