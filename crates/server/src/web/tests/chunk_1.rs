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

async fn login_cookie(app: &Router) -> String {
    let response = app
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
    response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
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

#[test]
fn message_query_limit_is_bounded() {
    assert_eq!(
        message_limit(&MessagesQuery { limit: None }),
        DEFAULT_MESSAGE_LIMIT
    );
    assert_eq!(message_limit(&MessagesQuery { limit: Some(250) }), 250);
    assert_eq!(message_limit(&MessagesQuery { limit: Some(0) }), 1);
    assert_eq!(
        message_limit(&MessagesQuery {
            limit: Some(10_000)
        }),
        MAX_MESSAGE_LIMIT
    );
}

#[tokio::test]
async fn prompt_file_requires_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(test_state(dir.path().join("config.yaml")));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/prompt-file?path=safety.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn prompt_file_can_be_read_and_updated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("safety.md"), "Original prompt").unwrap();
    let mut config = config();
    config.prompts.root = dir.path().to_path_buf();
    let app = router(AppState::new(
        dir.path().join("config.yaml"),
        config,
        "panel-key".to_string(),
    ));
    let cookie = login_cookie(&app).await;

    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/prompt-file?path=safety.md")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);
    let body = response_body(get_response).await;
    assert_eq!(body["path"], "safety.md");
    assert_eq!(body["content"], "Original prompt");

    let put_response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/prompt-file?path=safety.md")
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PromptFileRequest {
                        content: "Updated prompt".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("safety.md")).unwrap(),
        "Updated prompt"
    );
}

#[tokio::test]
async fn prompt_file_rejects_escape_paths_and_empty_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config();
    config.prompts.root = dir.path().to_path_buf();
    config.ai.review.prompt_path = "../config.yaml".into();
    let app = router(AppState::new(
        dir.path().join("config.yaml"),
        config,
        "panel-key".to_string(),
    ));
    let cookie = login_cookie(&app).await;

    let escape_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/prompt-file?path=..%2Fconfig.yaml")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(escape_response.status(), StatusCode::BAD_REQUEST);

    let unconfigured_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/prompt-file?path=missing.md")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unconfigured_response.status(), StatusCode::BAD_REQUEST);

    let empty_response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/prompt-file?path=safety.md")
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PromptFileRequest {
                        content: "  \n".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(empty_response.status(), StatusCode::BAD_REQUEST);
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
