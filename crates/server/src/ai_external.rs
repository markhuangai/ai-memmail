use std::time::Duration;

use serde_json::Value;

use crate::ai::{chat_completions_url, AiError};
use crate::config::AppConfig;

#[cfg(test)]
pub(crate) mod http_tests;

pub(crate) fn call_openai_chat(
    config: &AppConfig,
    messages: Vec<Value>,
) -> Result<String, AiError> {
    let url = chat_completions_url(&config.ai.api_url);
    let mut response = ureq::post(&url)
        .header("Authorization", &format!("Bearer {}", config.ai.api_secret))
        .header("Content-Type", "application/json")
        .config()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .send_json(serde_json::json!({
            "model": config.ai.model,
            "messages": messages,
            "temperature": 0,
        }))
        .map_err(|error| {
            AiError::Provider(protected_http_detail(
                &error.to_string(),
                &config.ai.api_secret,
            ))
        })?;
    if response.status().is_client_error() || response.status().is_server_error() {
        let code = response.status().as_u16();
        let text = response_error_text(&mut response);
        return Err(provider_error(code, &text));
    }
    let value: Value = response
        .body_mut()
        .read_json()
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
    let mut response = ureq::post(url)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .send_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .map_err(|error| AiError::Mcp(protected_http_detail(&error.to_string(), api_key)))?;
    if response.status().is_client_error() || response.status().is_server_error() {
        let code = response.status().as_u16();
        return Err(AiError::Mcp(http_error_summary(code)));
    }
    let value: Value = response
        .body_mut()
        .read_json()
        .map_err(|error| AiError::Mcp(error.to_string()))?;
    if let Some(error) = value.get("error") {
        let cause = match error.get("code").and_then(Value::as_i64) {
            Some(-32700) => "parse error",
            Some(-32600) => "invalid request",
            Some(-32601) => "method not found",
            Some(-32602) => "invalid params",
            Some(-32603) => "internal error",
            Some(-32099..=-32000) => "server error",
            _ => "upstream failure",
        };
        return Err(AiError::Mcp(format!(
            "MCP JSON-RPC error: {cause} (response body omitted)"
        )));
    }
    Ok(value)
}

fn provider_error(code: u16, text: &str) -> AiError {
    if is_context_length_error(code, text) {
        AiError::ContextLengthExceeded(
            "provider rejected the request because its context limit was exceeded".to_string(),
        )
    } else {
        let value = serde_json::from_str::<Value>(text).ok();
        let error_code = value
            .as_ref()
            .and_then(|value| value.pointer("/error/code"))
            .and_then(Value::as_str);
        let cause = match error_code {
            Some("invalid_api_key") => "invalid API key",
            Some("model_not_found") => "model not found",
            Some("insufficient_quota") => "insufficient quota",
            Some("rate_limit_exceeded") => "rate limit exceeded",
            _ => "upstream failure",
        };
        AiError::Provider(format!("{}; {cause}", http_error_summary(code)))
    }
}

fn http_error_summary(code: u16) -> String {
    let reason = ureq::http::StatusCode::from_u16(code)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("unknown status");
    format!("HTTP {code}: {reason} (response body omitted)")
}

fn response_error_text(response: &mut ureq::http::Response<ureq::Body>) -> String {
    match response.body_mut().read_to_string() {
        Ok(text) => text,
        Err(error) => format!("failed to read HTTP error body: {error}"),
    }
}

fn protected_http_detail(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, crate::config::REDACTED_SECRET)
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
