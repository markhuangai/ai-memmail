use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use axum::extract::State;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;
use uuid::Uuid;

use crate::config::{AppConfig, ConfigError};
use crate::storage::{PgStore, ProcessedEmail, StorageError};
use crate::worker;

const SESSION_COOKIE: &str = "ai_memmail_session";
const SESSION_TTL_SECONDS: u64 = 86_400;

#[derive(Clone)]
pub struct AppState {
    config_path: PathBuf,
    config: Arc<RwLock<AppConfig>>,
    sessions: SessionStore,
    control_panel_key: String,
    started_at: SystemTime,
}

#[derive(Clone, Default)]
struct SessionStore {
    tokens: Arc<Mutex<HashMap<String, SystemTime>>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusResponse {
    pub service: &'static str,
    pub authenticated: bool,
    pub uptime_seconds: u64,
    pub enabled_mailboxes: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginRequest {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginResponse {
    pub authenticated: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigResponse {
    pub config: AppConfig,
}

#[derive(Debug, Serialize)]
pub struct MessagesResponse {
    pub messages: Vec<ProcessedEmail>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

pub async fn serve(
    bind: SocketAddr,
    config_path: PathBuf,
    config: AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let control_panel_key = control_panel_key_from_env()?;
    let app = router(AppState::new(config_path, config, control_panel_key));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/config", get(get_config).put(update_config))
        .route("/api/messages", get(get_messages))
        .with_state(state)
        .fallback_service(ServeDir::new("web/dist").append_index_html_on_directories(true))
}

impl AppState {
    pub fn new(config_path: PathBuf, config: AppConfig, control_panel_key: String) -> Self {
        Self {
            config_path,
            config: Arc::new(RwLock::new(config)),
            sessions: SessionStore::default(),
            control_panel_key,
            started_at: SystemTime::now(),
        }
    }

    fn redacted_config(&self) -> AppConfig {
        self.config.read().expect("config lock poisoned").redacted()
    }

    fn replace_config(&self, mut config: AppConfig) -> Result<(), ConfigError> {
        let current = self.config.read().expect("config lock poisoned").clone();
        config.preserve_redacted_secrets(&current);
        config.save(&self.config_path)?;
        *self.config.write().expect("config lock poisoned") = config;
        Ok(())
    }

    fn create_session(&self) -> String {
        let token = Uuid::new_v4().to_string();
        let now = SystemTime::now();
        self.sessions
            .tokens
            .lock()
            .expect("session lock poisoned")
            .insert(
                token.clone(),
                now + Duration::from_secs(SESSION_TTL_SECONDS),
            );
        token
    }

    fn remove_session(&self, token: &str) {
        self.sessions
            .tokens
            .lock()
            .expect("session lock poisoned")
            .remove(token);
    }

    fn has_session(&self, token: &str) -> bool {
        let now = SystemTime::now();
        let mut tokens = self.sessions.tokens.lock().expect("session lock poisoned");
        tokens.retain(|_, expires_at| *expires_at > now);
        tokens.contains_key(token)
    }
}

async fn status(State(state): State<AppState>, headers: HeaderMap) -> Json<StatusResponse> {
    let uptime_seconds = state
        .started_at
        .elapsed()
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let config = state.config.read().expect("config lock poisoned");
    Json(StatusResponse {
        service: "ai-memmail",
        authenticated: authenticated(&state, &headers),
        uptime_seconds,
        enabled_mailboxes: worker::poll_plans(&config).len(),
    })
}

async fn login(State(state): State<AppState>, Json(request): Json<LoginRequest>) -> Response {
    if request.key != state.control_panel_key {
        return ApiError::unauthorized("invalid control panel key").into_response();
    }
    let token = state.create_session();
    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL_SECONDS}"
    );
    (
        StatusCode::OK,
        [(SET_COOKIE, cookie)],
        Json(LoginResponse {
            authenticated: true,
        }),
    )
        .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        state.remove_session(&token);
    }
    (
        StatusCode::OK,
        [(
            SET_COOKIE,
            format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
        Json(LoginResponse {
            authenticated: false,
        }),
    )
        .into_response()
}

async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    Ok(Json(ConfigResponse {
        config: state.redacted_config(),
    }))
}

async fn update_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(config): Json<AppConfig>,
) -> Result<Json<ConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    state
        .replace_config(config)
        .map_err(ApiError::from_config)?;
    Ok(Json(ConfigResponse {
        config: state.redacted_config(),
    }))
}

async fn get_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MessagesResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    store.migrate().await.map_err(ApiError::from_storage)?;
    let messages = store
        .list_processed_emails(100)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(MessagesResponse { messages }))
}

fn control_panel_key_from_env() -> Result<String, ConfigError> {
    let key = std::env::var("CONTROL_PANEL_KEY").map_err(|_| {
        ConfigError::Invalid("CONTROL_PANEL_KEY environment variable is required".to_string())
    })?;
    if key.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "CONTROL_PANEL_KEY environment variable is required".to_string(),
        ));
    }
    Ok(key)
}

fn require_auth(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if authenticated(state, headers) {
        Ok(())
    } else {
        Err(ApiError::unauthorized("control panel login required"))
    }
}

fn authenticated(state: &AppState, headers: &HeaderMap) -> bool {
    session_token(headers)
        .as_deref()
        .map(|token| state.has_session(token))
        .unwrap_or(false)
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_string())
    })
}

impl ApiError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn from_config(error: ConfigError) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }

    fn from_storage(error: StorageError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::body::Body;
    use axum::http::{header::SET_COOKIE, Method, Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::config::{
        AgentConfig, AiConfig, AiProtocol, DatabaseConfig, ImapConfig, LoggingConfig,
        MailboxConfig, McpServerConfig, McpTransport, PromptConfig, ReviewConfig, SmtpConfig,
    };

    use super::*;

    fn config() -> AppConfig {
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
            mcp_servers: BTreeMap::from([(
                "dense_mem".to_string(),
                McpServerConfig {
                    transport: McpTransport::Stdio,
                    command: Some("npx".to_string()),
                    args: vec![],
                    env: BTreeMap::from([
                        ("DENSE_MEM_PASSWORD".to_string(), "mcp-password".to_string()),
                        (
                            "DENSE_MEM_MCP_URL".to_string(),
                            "http://dense-mem".to_string(),
                        ),
                    ]),
                    url: None,
                },
            )]),
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

    fn test_state(path: PathBuf) -> AppState {
        AppState::new(path, config(), "panel-key".to_string())
    }

    async fn response_body(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn login_rejects_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"wrong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn config_requires_login() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn messages_requires_login() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn status_reports_session_state_and_mailbox_count() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert_eq!(body["service"], "ai-memmail");
        assert_eq!(body["authenticated"], false);
        assert_eq!(body["enabled_mailboxes"], 1);
    }

    #[tokio::test]
    async fn authenticated_config_is_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"panel-key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/config")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert_eq!(body["config"]["database"]["password"], "********");
        assert_eq!(body["config"]["ai"]["AI_API_SECRET"], "********");
        assert_eq!(
            body["config"]["mailboxes"][0]["imap"]["password"],
            "********"
        );
        assert_eq!(
            body["config"]["mcp_servers"]["dense_mem"]["env"]["DENSE_MEM_PASSWORD"],
            "********"
        );
        assert_eq!(
            body["config"]["mcp_servers"]["dense_mem"]["env"]["DENSE_MEM_MCP_URL"],
            "http://dense-mem"
        );
    }

    #[tokio::test]
    async fn login_cookie_uses_server_session_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"panel-key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains(&format!("Max-Age={SESSION_TTL_SECONDS}")));
    }

    #[test]
    fn expired_sessions_are_rejected_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path().join("config.yaml"));
        let active = state.create_session();
        let expired = state.create_session();
        {
            let mut tokens = state.sessions.tokens.lock().expect("session lock poisoned");
            tokens.insert(expired.clone(), SystemTime::now() - Duration::from_secs(1));
        }

        assert!(!state.has_session(&expired));
        assert!(state.has_session(&active));
        assert!(!state
            .sessions
            .tokens
            .lock()
            .expect("session lock poisoned")
            .contains_key(&expired));
    }

    #[tokio::test]
    async fn logout_removes_session() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"panel-key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let logout_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/logout")
                    .header(COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout_response.status(), StatusCode::OK);

        let config_response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/config")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(config_response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn update_config_validates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let state = test_state(path.clone());
        let app = router(state);
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"panel-key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mut next = config();
        next.mailboxes[0].poll_interval_seconds = 45;
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config")
                    .header(COOKIE, cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&next).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved = AppConfig::load(&path).unwrap();
        assert_eq!(saved.mailboxes[0].poll_interval_seconds, 45);
    }

    #[tokio::test]
    async fn update_config_preserves_redacted_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let state = test_state(path.clone());
        let app = router(state);
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"panel-key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/config")
                    .header(COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let body = response_body(get_response).await;
        let mut next: ConfigResponse = serde_json::from_value(body).unwrap();
        assert_eq!(next.config.database.password, "********");
        assert_eq!(next.config.ai.api_secret, "********");

        next.config.database.host = "db.changed.test".to_string();
        next.config.ai.model = "changed-model".to_string();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config")
                    .header(COOKIE, cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&next.config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let saved = AppConfig::load(&path).unwrap();
        assert_eq!(saved.database.host, "db.changed.test");
        assert_eq!(saved.database.password, "db-secret");
        assert_eq!(saved.ai.api_secret, "secret");
        assert_eq!(saved.mailboxes[0].imap.password, "imap-secret");
        assert_eq!(saved.mailboxes[0].smtp.password, "smtp-secret");
    }

    #[tokio::test]
    async fn update_config_rejects_invalid_payload() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state(dir.path().join("config.yaml")));
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"panel-key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mut next = config();
        next.database.host.clear();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config")
                    .header(COOKIE, cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&next).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("database.host"));
    }

    #[test]
    fn control_panel_key_requires_non_empty_env() {
        std::env::remove_var("CONTROL_PANEL_KEY");
        let missing = control_panel_key_from_env().unwrap_err().to_string();
        assert!(missing.contains("CONTROL_PANEL_KEY"));

        std::env::set_var("CONTROL_PANEL_KEY", " ");
        let empty = control_panel_key_from_env().unwrap_err().to_string();
        assert!(empty.contains("CONTROL_PANEL_KEY"));

        std::env::set_var("CONTROL_PANEL_KEY", "panel-key");
        assert_eq!(control_panel_key_from_env().unwrap(), "panel-key");
        std::env::remove_var("CONTROL_PANEL_KEY");
    }

    #[test]
    fn extracts_session_token_from_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            "theme=dark; ai_memmail_session=abc; other=1"
                .parse()
                .unwrap(),
        );
        assert_eq!(session_token(&headers), Some("abc".to_string()));
    }

    #[test]
    fn missing_or_malformed_cookie_has_no_session_token() {
        let headers = HeaderMap::new();
        assert_eq!(session_token(&headers), None);

        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "bad-cookie".parse().unwrap());
        assert_eq!(session_token(&headers), None);
    }
}
