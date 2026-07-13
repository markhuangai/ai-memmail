use std::time::Duration;

use serde_json::Value;

use crate::ai::{chat_completions_url, truncate_chars, AiError};
use crate::config::AppConfig;

pub(crate) fn call_openai_chat(
    config: &AppConfig,
    messages: Vec<Value>,
) -> Result<String, AiError> {
    let url = chat_completions_url(&config.ai.api_url);
    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", config.ai.api_secret))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(60))
        .send_json(serde_json::json!({
            "model": config.ai.model,
            "messages": messages,
            "temperature": 0,
        }))
        .map_err(provider_error)?;
    let value: Value = response
        .into_json()
        .map_err(|error| AiError::Provider(error.to_string()))?;
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| AiError::Provider("chat response missing message content".to_string()))
}

pub(crate) fn mcp_http_call(
    url: &str,
    api_key: &str,
    method: &str,
    params: Value,
) -> Result<Value, AiError> {
    let response = ureq::post(url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(30))
        .send_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .map_err(|error| AiError::Mcp(http_error_message(error)))?;
    let value: Value = response
        .into_json()
        .map_err(|error| AiError::Mcp(error.to_string()))?;
    if let Some(error) = value.get("error") {
        return Err(AiError::Mcp(format!("MCP JSON-RPC error: {error}")));
    }
    Ok(value)
}

fn provider_error(error: ureq::Error) -> AiError {
    match error {
        ureq::Error::Status(code, response) => {
            let text = response
                .into_string()
                .unwrap_or_else(|read_error| read_error.to_string());
            if is_context_length_error(code, &text) {
                AiError::ContextLengthExceeded(
                    "provider rejected the request because its context limit was exceeded"
                        .to_string(),
                )
            } else {
                AiError::Provider(format!("HTTP {code}: {}", truncate_chars(&text, 500)))
            }
        }
        ureq::Error::Transport(error) => AiError::Provider(error.to_string()),
    }
}

pub(crate) fn is_context_length_error(code: u16, body: &str) -> bool {
    if !matches!(code, 400 | 413 | 422) {
        return false;
    }
    if code == 413 {
        return true;
    }
    let body = body.to_ascii_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "context window",
        "too many tokens",
        "prompt is too long",
        "input is too long",
    ]
    .iter()
    .any(|needle| body.contains(needle))
}

fn http_error_message(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let text = response
                .into_string()
                .unwrap_or_else(|read_error| read_error.to_string());
            format!("HTTP {code}: {}", truncate_chars(&text, 500))
        }
        ureq::Error::Transport(error) => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::is_context_length_error;

    #[test]
    fn recognizes_provider_context_limit_errors_only_for_relevant_statuses() {
        assert!(is_context_length_error(
            400,
            r#"{"error":{"code":"context_length_exceeded"}}"#
        ));
        assert!(is_context_length_error(413, "payload rejected"));
        assert!(!is_context_length_error(
            500,
            "Maximum context length exceeded"
        ));
        assert!(!is_context_length_error(400, "invalid model"));
    }
}
