pub fn deterministic_safety_scan(message: &InboundMessage) -> Option<SafetyScanResult> {
    let content = format!(
        "{}\n{}",
        message.metadata.subject.to_ascii_lowercase(),
        message.plain_text.to_ascii_lowercase()
    );
    let jailbreak_terms = [
        "ignore previous instructions",
        "ignore all previous instructions",
        "reveal your system prompt",
        "developer message",
        "config.yaml",
        "local.yaml",
        "api key",
        "password",
        "jailbreak",
    ];
    if jailbreak_terms.iter().any(|term| content.contains(term)) {
        return Some(SafetyScanResult {
            category: SafetyCategory::PromptInjection,
            reason: "message contains prompt-injection or secret-exfiltration language".to_string(),
            confidence: 0.95,
        });
    }
    let hacking_terms = [
        "write malware",
        "phishing kit",
        "steal credentials",
        "bypass authentication",
        "exploit this server",
    ];
    if hacking_terms.iter().any(|term| content.contains(term)) {
        return Some(SafetyScanResult {
            category: SafetyCategory::Hacking,
            reason: "message requests malicious technical behavior".to_string(),
            confidence: 0.9,
        });
    }
    None
}

pub fn human_review_requested(message: &InboundMessage) -> bool {
    let content = format!(
        "{}\n{}",
        message.metadata.subject.to_ascii_lowercase(),
        message.plain_text.to_ascii_lowercase()
    );
    [
        "forward to a human",
        "forward this to a human",
        "human review",
        "manual review",
        "escalation to human",
        "escalate to human",
        "please escalate",
    ]
    .iter()
    .any(|needle| content.contains(needle))
}

pub fn forward_decision(
    mailbox: &MailboxConfig,
    message: &InboundMessage,
    reason: impl Into<String>,
) -> AgentDecision {
    let recipients = if mailbox.agent.default_forward_to.is_empty() {
        mailbox.safety_forward_to.clone()
    } else {
        mailbox.agent.default_forward_to.clone()
    };
    AgentDecision {
        action: OutboundAction {
            kind: crate::mail::OutboundActionKind::Forward,
            recipients,
            subject: format!("Fwd: {}", message.metadata.subject),
            body: format!(
                "Human review requested for message from {}.",
                message.metadata.from_addr
            ),
            reason: reason.into(),
            message_id: None,
            in_reply_to: None,
            references: vec![],
        },
        safety_notes: "message passed deterministic human-review routing".to_string(),
    }
}

pub async fn recall_mailbox_context(
    config: &AppConfig,
    mailbox: &MailboxConfig,
    message: &InboundMessage,
) -> Result<String, AiError> {
    HttpMcpContextProvider::default()
        .recall_mailbox_context(config, mailbox, message)
        .await
}

async fn recall_from_mcp_server<C>(
    client: C,
    server_name: &str,
    server: &McpServerConfig,
    message: &InboundMessage,
) -> Result<String, AiError>
where
    C: BlockingMcpClient,
{
    let url = server
        .url
        .clone()
        .or_else(|| server.env.get("DENSE_MEM_MCP_URL").cloned())
        .ok_or_else(|| AiError::Mcp(format!("{server_name} missing MCP URL")))?;
    let api_key = server
        .env
        .get("DENSE_MEM_API_KEY")
        .cloned()
        .ok_or_else(|| AiError::Mcp(format!("{server_name} missing MCP API key")))?;
    let query = mcp_recall_query(message);
    let server_name = server_name.to_string();
    tokio::task::spawn_blocking(move || {
        let assemble_query = query.clone();
        let response = client
            .call(
                &url,
                &api_key,
                "tools/call",
                serde_json::json!({
                    "name": "assemble_context",
                    "arguments": {
                        "query": assemble_query,
                        "limit": 5,
                        "max_chars": 4000
                    }
                }),
            )
            .or_else(|_| {
                client.call(
                    &url,
                    &api_key,
                    "tools/call",
                    serde_json::json!({
                        "name": "recall_memory",
                        "arguments": {
                            "query": query,
                            "limit": 5
                        }
                    }),
                )
            })?;
        Ok(format!(
            "MCP server {server_name} memory context result:\n{}",
            mcp_result_text(&response)
        ))
    })
    .await
    .map_err(|error| AiError::Mcp(error.to_string()))?
}

fn mcp_result_text(response: &Value) -> String {
    let content = response["result"]["content"].as_array();
    if let Some(content) = content {
        let text = content
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            return truncate_chars(&text, 8_000);
        }
    }
    truncate_chars(&response.to_string(), 8_000)
}

fn chat_message(role: &str, content: String) -> Value {
    serde_json::json!({ "role": role, "content": content })
}

fn bounded_chat_messages(system: String, user: String) -> Result<Vec<Value>, AiError> {
    let messages = vec![
        chat_message("system", system),
        chat_message("user", user),
    ];
    let serialized_chars = serde_json::to_string(&messages)
        .map_err(|error| AiError::Provider(error.to_string()))?
        .chars()
        .count();
    if serialized_chars > MAX_SERIALIZED_PROMPT_CHARS {
        return Err(AiError::ContextLengthExceeded(format!(
            "serialized prompt has {serialized_chars} characters; limit is {MAX_SERIALIZED_PROMPT_CHARS}"
        )));
    }
    Ok(messages)
}

pub(crate) fn chat_completions_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if index >= max_chars {
            output.push_str(TRUNCATION_MARKER);
            break;
        }
        output.push(character);
    }
    output
}

fn mcp_recall_query(message: &InboundMessage) -> String {
    let query = format!(
        "From: {}\nSubject: {}\n\n{}",
        message.metadata.from_addr, message.metadata.subject, message.plain_text
    );
    truncate_chars_to_limit(&query, MCP_QUERY_MAX_CHARS)
}

fn truncate_chars_to_limit(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let marker_chars = TRUNCATION_MARKER.chars().count();
    if max_chars <= marker_chars {
        return value.chars().take(max_chars).collect();
    }

    let keep_chars = max_chars - marker_chars;
    let mut output = value.chars().take(keep_chars).collect::<String>();
    output.push_str(TRUNCATION_MARKER);
    output
}

fn json_object_text(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return trimmed;
    }
    match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if start < end => &trimmed[start..=end],
        _ => trimmed,
    }
}

pub fn validate_agent_decision(decision: &AgentDecision) -> Result<(), Vec<ValidationError>> {
    let mut errors = match validate_outbound_action(&decision.action) {
        Ok(()) => Vec::new(),
        Err(errors) => errors,
    };
    if matches!(
        decision.action.kind,
        OutboundActionKind::Reply | OutboundActionKind::Forward
    ) && reply_body_uses_system_voice(&decision.action.body)
    {
        errors.push(ValidationError {
            field: "body".to_string(),
            message: "reply body must not present the sender as an AI or mailbox agent".to_string(),
        });
    }
    if decision.safety_notes.trim().is_empty() {
        errors.push(ValidationError {
            field: "safety_notes".to_string(),
            message: "safety_notes is required".to_string(),
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn reply_body_uses_system_voice(body: &str) -> bool {
    let normalized = body.to_ascii_lowercase().replace(['\n', '\r'], " ");
    [
        "as an ai",
        "as an assistant",
        "as a bot",
        "i am an ai",
        "i'm an ai",
        "ai assistant",
        "i am an assistant",
        "i'm an assistant",
        "i am mark's email assistant",
        "i'm mark's email assistant",
        "i am your email assistant",
        "i'm your email assistant",
        "i am a bot",
        "i'm a bot",
        "bot handling this",
        "mailbox agent",
        "email-processing agent",
        "i can help handle mail",
        "sent to this mailbox",
        "this mailbox",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}
