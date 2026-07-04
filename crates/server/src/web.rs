use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use axum::extract::{Path, Query, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;
use uuid::Uuid;

use crate::classification::{EmailClassificationConfig, NewEmailRule};
use crate::config::{AppConfig, ConfigError};
use crate::prompts;
use crate::storage::{PgStore, ProcessedEmail, ProcessingStore, StorageError};
use crate::worker;

const SESSION_COOKIE: &str = "ai_memmail_session";
const SESSION_TTL_SECONDS: u64 = 86_400;
const DEFAULT_MESSAGE_LIMIT: i64 = 100;
const MAX_MESSAGE_LIMIT: i64 = 500;

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

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct MessagesQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct PromptPathQuery {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptFileResponse {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptFileRequest {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct EmailClassificationConfigResponse {
    pub classification: EmailClassificationConfig,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct LabelRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
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
        .route(
            "/api/prompt-file",
            get(get_prompt_file).put(update_prompt_file),
        )
        .route("/api/email-classification", get(get_email_classification))
        .route("/api/email-categories", post(create_email_category))
        .route("/api/email-topics", post(create_email_topic))
        .route("/api/email-rules", post(create_email_rule))
        .route(
            "/api/email-rules/:id",
            put(update_email_rule).delete(delete_email_rule),
        )
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
    Query(query): Query<MessagesQuery>,
    headers: HeaderMap,
) -> Result<Json<MessagesResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let limit = message_limit(&query);
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    let messages = store
        .list_processed_emails(limit)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(MessagesResponse { messages }))
}

async fn get_prompt_file(
    State(state): State<AppState>,
    Query(query): Query<PromptPathQuery>,
    headers: HeaderMap,
) -> Result<Json<PromptFileResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let resolved = resolve_prompt_file(&config, &query.path)?;
    let content = fs::read_to_string(&resolved).map_err(|source| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: format!("failed to read prompt {}: {source}", resolved.display()),
    })?;
    Ok(Json(PromptFileResponse {
        path: query.path,
        content,
    }))
}

async fn update_prompt_file(
    State(state): State<AppState>,
    Query(query): Query<PromptPathQuery>,
    headers: HeaderMap,
    Json(request): Json<PromptFileRequest>,
) -> Result<Json<PromptFileResponse>, ApiError> {
    require_auth(&state, &headers)?;
    if request.content.trim().is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "prompt content must not be empty".to_string(),
        });
    }
    let config = state.config.read().expect("config lock poisoned").clone();
    let resolved = resolve_prompt_file(&config, &query.path)?;
    fs::write(&resolved, &request.content).map_err(|source| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: format!("failed to write prompt {}: {source}", resolved.display()),
    })?;
    Ok(Json(PromptFileResponse {
        path: query.path,
        content: request.content,
    }))
}

fn resolve_prompt_file(config: &AppConfig, path: &str) -> Result<PathBuf, ApiError> {
    let prompt_path = FsPath::new(path);
    if !prompt_path_is_configured(config, prompt_path) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("prompt path is not configured: {path}"),
        });
    }
    prompts::resolve_prompt_path(&config.prompts.root, prompt_path).map_err(|error| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: error.to_string(),
    })
}

fn prompt_path_is_configured(config: &AppConfig, path: &FsPath) -> bool {
    config.prompts.safety_scan.as_path() == path
        || config.prompts.email_classifier.as_path() == path
        || config.prompts.rule_action.as_path() == path
        || config.ai.review.prompt_path.as_path() == path
        || config
            .mailboxes
            .iter()
            .any(|mailbox| mailbox.agent.system_prompt_path.as_path() == path)
}

fn message_limit(query: &MessagesQuery) -> i64 {
    query
        .limit
        .unwrap_or(DEFAULT_MESSAGE_LIMIT)
        .clamp(1, MAX_MESSAGE_LIMIT)
}

async fn get_email_classification(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<EmailClassificationConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    Ok(Json(EmailClassificationConfigResponse {
        classification: load_email_classification_config(&state).await?,
    }))
}

async fn create_email_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LabelRequest>,
) -> Result<Json<EmailClassificationConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let store = connect_store(&state).await?;
    store
        .create_email_category(&request.name, &request.description)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(EmailClassificationConfigResponse {
        classification: load_email_classification_config(&state).await?,
    }))
}

async fn create_email_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LabelRequest>,
) -> Result<Json<EmailClassificationConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let store = connect_store(&state).await?;
    store
        .create_email_topic(&request.name, &request.description)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(EmailClassificationConfigResponse {
        classification: load_email_classification_config(&state).await?,
    }))
}

async fn create_email_rule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(rule): Json<NewEmailRule>,
) -> Result<Json<EmailClassificationConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let store = connect_store(&state).await?;
    store
        .create_email_rule(rule)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(EmailClassificationConfigResponse {
        classification: load_email_classification_config(&state).await?,
    }))
}

async fn update_email_rule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(rule): Json<NewEmailRule>,
) -> Result<Json<EmailClassificationConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let store = connect_store(&state).await?;
    store
        .update_email_rule(id, rule)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(EmailClassificationConfigResponse {
        classification: load_email_classification_config(&state).await?,
    }))
}

async fn delete_email_rule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<EmailClassificationConfigResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let store = connect_store(&state).await?;
    store
        .delete_email_rule(id)
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(EmailClassificationConfigResponse {
        classification: load_email_classification_config(&state).await?,
    }))
}

async fn load_email_classification_config(
    state: &AppState,
) -> Result<EmailClassificationConfig, ApiError> {
    let config = state.config.read().expect("config lock poisoned").clone();
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    store
        .ensure_default_classification_policy(&config)
        .await
        .map_err(ApiError::from_storage)?;
    store
        .list_email_classification_config()
        .await
        .map_err(ApiError::from_storage)
}

async fn connect_store(state: &AppState) -> Result<PgStore, ApiError> {
    let config = state.config.read().expect("config lock poisoned").clone();
    PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)
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
        let status = match &error {
            StorageError::ClassificationNotFound(_) => StatusCode::NOT_FOUND,
            StorageError::InvalidClassification(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
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
mod tests;
