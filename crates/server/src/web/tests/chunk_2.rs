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
