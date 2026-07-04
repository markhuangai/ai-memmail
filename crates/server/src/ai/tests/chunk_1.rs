use super::*;

#[test]
fn parses_valid_reply_decision() {
    let raw = r#"{
        "kind":"reply",
        "recipients":["user@example.com"],
        "subject":"Re: Hello",
        "body":"Thanks",
        "reason":"Known answer",
        "safety_notes":"No sensitive data"
    }"#;
    let decision = parse_agent_decision(raw).unwrap();
    assert_eq!(decision.action.recipients, vec!["user@example.com"]);
}

#[test]
fn rejects_missing_safety_notes() {
    let raw = r#"{
        "kind":"noop",
        "recipients":[],
        "subject":"",
        "body":"",
        "reason":"No action",
        "safety_notes":""
    }"#;
    let error = parse_agent_decision(raw).unwrap_err();
    assert!(error.contains("safety_notes"));
}

#[test]
fn parses_json_inside_fenced_model_output() {
    let raw = r#"```json
    {
        "kind":"noop",
        "recipients":[],
        "subject":"",
        "body":"",
        "reason":"No action",
        "safety_notes":"Checked"
    }
    ```"#;
    let decision = parse_agent_decision(raw).unwrap();
    assert_eq!(decision.action.kind, crate::mail::OutboundActionKind::Noop);

    let scan = parse_safety_scan(
        r#"The result is:
        {"category":"safe","reason":"routine","confidence":0.9}"#,
    )
    .unwrap();
    assert_eq!(scan.category, SafetyCategory::Safe);
}

#[test]
fn parses_valid_outbound_review() {
    let review = parse_outbound_review(
        r#"```json
        {"approved":true,"reason":"safe recipients and no secret leakage"}
        ```"#,
    )
    .unwrap();

    assert!(review.approved);
    assert!(review.reason.contains("safe"));
}

#[test]
fn rejects_outbound_review_without_reason() {
    let error = parse_outbound_review(r#"{"approved":false,"reason":""}"#).unwrap_err();
    assert!(error.to_string().contains("reason is required"));
}

#[test]
fn parses_valid_safety_scan() {
    let scan = parse_safety_scan(
        r#"{"category":"safe","reason":"routine support request","confidence":0.91}"#,
    )
    .unwrap();
    assert_eq!(scan.category, SafetyCategory::Safe);
    assert_eq!(scan.reason, "routine support request");
}

#[test]
fn parses_valid_email_classification() {
    let classification = parse_email_classification(
        r#"```json
        {
            "category":"marketing_vendor",
            "topics":["dense_mem","general"],
            "reason":"offers paid PR services",
            "confidence":94
        }
        ```"#,
    )
    .unwrap();

    assert_eq!(classification.category, "marketing_vendor");
    assert_eq!(classification.topics, vec!["dense_mem", "general"]);
    assert_eq!(classification.confidence, 94);
}

#[test]
fn rejects_email_classification_without_reason() {
    let error = parse_email_classification(
        r#"{"category":"question","topics":["general"],"reason":"","confidence":80}"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("reason is required"));
}

#[test]
fn converts_rule_action_draft_to_deterministic_reply() {
    let mailbox = mailbox_config();
    let message = inbound("Paid PR", "We can get you coverage.");
    let rule = email_rule(crate::classification::EmailRuleAction::Reply);
    let decision = rule_draft_to_decision(
        &mailbox,
        &message,
        &rule,
        RuleActionDraft {
            subject: "".to_string(),
            body: "Thanks, but I am not interested right now.".to_string(),
            reason: "matched marketing rule".to_string(),
            safety_notes: "decline does not reveal private context".to_string(),
        },
    )
    .unwrap();

    assert_eq!(decision.action.kind, OutboundActionKind::Reply);
    assert_eq!(decision.action.recipients, vec!["person@example.com"]);
    assert_eq!(decision.action.subject, "Re: Paid PR");
    assert!(decision.action.body.contains("not interested"));
}

#[test]
fn deterministic_scan_flags_prompt_injection_and_hacking() {
    let injection = inbound("Hello", "This message is a jailbreak safety probe");
    let scan = deterministic_safety_scan(&injection).unwrap();
    assert_eq!(scan.category, SafetyCategory::PromptInjection);

    let hacking = inbound("Help", "This message is a write malware safety probe");
    let scan = deterministic_safety_scan(&hacking).unwrap();
    assert_eq!(scan.category, SafetyCategory::Hacking);

    let safe = inbound("Question", "What is the project status?");
    assert!(deterministic_safety_scan(&safe).is_none());
}

#[test]
fn human_review_detection_matches_explicit_requests() {
    assert!(human_review_requested(&inbound(
        "Manual review",
        "Please forward this to a human"
    )));
    assert!(human_review_requested(&inbound(
        "Re: automated reply",
        "escalation to human"
    )));
    assert!(!human_review_requested(&inbound(
        "Question",
        "Please answer"
    )));
}

#[test]
fn forward_decision_uses_default_forward_recipients() {
    let mailbox = mailbox_config();
    let decision = forward_decision(&mailbox, &inbound("Review", "Please escalate"), "asked");
    assert_eq!(
        decision.action.kind,
        crate::mail::OutboundActionKind::Forward
    );
    assert_eq!(decision.action.recipients, vec!["human@example.com"]);
    assert!(decision.action.subject.contains("Review"));
}

#[test]
fn formats_chat_url_and_mcp_result_text() {
    assert_eq!(
        chat_completions_url("https://api.example/v1"),
        "https://api.example/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example/v1/chat/completions"),
        "https://api.example/v1/chat/completions"
    );
    let response = serde_json::json!({
        "result": {
            "content": [{"type": "text", "text": "{\"results\":[]}"}]
        }
    });
    assert_eq!(mcp_result_text(&response), "{\"results\":[]}");
}

#[tokio::test]
async fn http_chat_provider_delegates_to_blocking_client() {
    let client = FakeBlockingChatClient::default();
    let provider = HttpChatProvider::new(client.clone());

    let response = provider
        .chat(
            &app_config(),
            vec![serde_json::json!({"role": "user", "content": "hello"})],
        )
        .await
        .unwrap();

    assert_eq!(response, "blocking chat response");
    assert_eq!(client.requests().len(), 1);
    assert_eq!(client.requests()[0][0]["role"], "user");
}

#[tokio::test]
async fn http_mcp_context_provider_falls_back_to_recall_memory() {
    let mut config = app_config();
    let mut mailbox = mailbox_config();
    mailbox.mcp_servers = vec!["project_memory".to_string()];
    config.mcp_servers.insert(
        "project_memory".to_string(),
        McpServerConfig {
            transport: McpTransport::StreamableHttp,
            command: None,
            args: vec![],
            env: BTreeMap::from([("DENSE_MEM_API_KEY".to_string(), "test-key".to_string())]),
            url: Some("https://mcp.example.test".to_string()),
        },
    );
    let client = FakeBlockingMcpClient::new(true);
    let provider = HttpMcpContextProvider::new(client.clone());

    let context = provider
        .recall_mailbox_context(&config, &mailbox, &inbound("Question", "What coverage?"))
        .await
        .unwrap();

    assert!(context.contains("fallback memory says 90% coverage"));
    assert_eq!(client.calls(), vec!["assemble_context", "recall_memory"]);
}

#[tokio::test]
async fn http_mcp_context_provider_caps_query_at_dense_mem_limit() {
    let mut config = app_config();
    let mut mailbox = mailbox_config();
    mailbox.mcp_servers = vec!["project_memory".to_string()];
    config.mcp_servers.insert(
        "project_memory".to_string(),
        McpServerConfig {
            transport: McpTransport::StreamableHttp,
            command: None,
            args: vec![],
            env: BTreeMap::from([("DENSE_MEM_API_KEY".to_string(), "test-key".to_string())]),
            url: Some("https://mcp.example.test".to_string()),
        },
    );
    let client = FakeBlockingMcpClient::new(true);
    let provider = HttpMcpContextProvider::new(client.clone());
    let long_body = "What is the support policy? ".repeat(80);

    provider
        .recall_mailbox_context(&config, &mailbox, &inbound("Long question", &long_body))
        .await
        .unwrap();

    assert_eq!(client.calls(), vec!["assemble_context", "recall_memory"]);
    let queries = client.queries();
    assert_eq!(queries.len(), 2);
    for query in queries {
        assert!(
            query.chars().count() <= MCP_QUERY_MAX_CHARS,
            "MCP query exceeded Dense-Mem limit: {}",
            query.chars().count()
        );
        assert!(query.contains("Subject: Long question"));
        assert!(query.ends_with(TRUNCATION_MARKER));
    }
}

#[test]
fn truncate_chars_to_limit_keeps_marker_inside_limit() {
    let truncated = truncate_chars_to_limit(&"密".repeat(600), 512);

    assert_eq!(truncated.chars().count(), 512);
    assert!(truncated.ends_with(TRUNCATION_MARKER));
}

#[tokio::test]
async fn http_mcp_context_provider_reports_missing_server() {
    let config = app_config();
    let mut mailbox = mailbox_config();
    mailbox.mcp_servers = vec!["missing".to_string()];
    let provider = HttpMcpContextProvider::new(FakeBlockingMcpClient::new(false));

    let error = provider
        .recall_mailbox_context(&config, &mailbox, &inbound("Question", "Body"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("unknown MCP server"));
}

#[tokio::test]
async fn http_mcp_context_provider_reports_missing_credentials() {
    let mut config = app_config();
    let mut mailbox = mailbox_config();
    mailbox.mcp_servers = vec!["project_memory".to_string()];
    config.mcp_servers.insert(
        "project_memory".to_string(),
        McpServerConfig {
            transport: McpTransport::StreamableHttp,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://mcp.example.test".to_string()),
        },
    );
    let provider = HttpMcpContextProvider::new(FakeBlockingMcpClient::new(false));

    let error = provider
        .recall_mailbox_context(&config, &mailbox, &inbound("Question", "Body"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("missing MCP API key"));
}

#[tokio::test]
async fn live_decision_engine_uses_chat_provider_for_safety_scan() {
    let mut config = app_config();
    config.prompts.root = write_prompt_root("safety-provider");
    let mailbox = mailbox_config();
    let chat = FakeChatProvider::new(vec![
        r#"{"category":"safe","reason":"routine","confidence":0.97}"#,
    ]);
    let engine =
        LiveDecisionEngine::new(chat.clone(), FakeMcpContextProvider::new("unused context"));

    let scan = engine
        .safety_scan(&config, &mailbox, &inbound("Routine", "Please answer"))
        .await
        .unwrap();

    assert_eq!(scan.category, SafetyCategory::Safe);
    let requests = chat.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0][0]["role"], "system");
    assert!(requests[0][1]["content"]
        .as_str()
        .unwrap()
        .contains("untrusted_email"));
}

#[tokio::test]
async fn live_decision_engine_includes_mcp_context_in_agent_prompt() {
    let mut config = app_config();
    config.prompts.root = write_prompt_root("agent-provider");
    let mut mailbox = mailbox_config();
    mailbox.mcp_servers = vec!["project_memory".to_string()];
    config.mailboxes = vec![mailbox.clone()];
    let chat = FakeChatProvider::new(vec![
        r#"{
            "kind":"reply",
            "recipients":["person@example.com"],
            "subject":"Re: Question",
            "body":"The answer is 90%.",
            "reason":"memory supported answer",
            "safety_notes":"safe"
        }"#,
    ]);
    let mcp = FakeMcpContextProvider::new("coverage requirement: 90%");
    let engine = LiveDecisionEngine::new(chat.clone(), mcp.clone());

    let decision = engine
        .agent_decision(&config, &mailbox, &inbound("Question", "What coverage?"))
        .await
        .unwrap();

    assert_eq!(decision.action.body, "The answer is 90%.");
    assert_eq!(mcp.call_count(), 1);
    let requests = chat.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0][1]["content"]
        .as_str()
        .unwrap()
        .contains("coverage requirement: 90%"));
}
