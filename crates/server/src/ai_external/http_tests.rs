use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

use super::*;

fn config(url: String) -> AppConfig {
    let mut config = AppConfig::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/config.example.yaml"),
    )
    .unwrap();
    config.ai.api_url = url;
    config.ai.api_secret = "synthetic-http-credential".to_string();
    config
}

pub(crate) fn server(code: u16, body: &str) -> (String, JoinHandle<(String, Value)>) {
    server_for_calls(code, body, 1)
}

pub(crate) fn server_for_calls(
    code: u16,
    body: &str,
    calls: usize,
) -> (String, JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..calls {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut length = None;
            let mut chunked = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                let (key, value) = line.split_once(':').unwrap();
                if key.eq_ignore_ascii_case("content-length") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
                if key.eq_ignore_ascii_case("transfer-encoding") {
                    chunked = value.trim().eq_ignore_ascii_case("chunked");
                }
                if key.eq_ignore_ascii_case("authorization") {
                    assert_eq!(value.trim(), "Bearer synthetic-http-credential");
                }
            }
            let mut bytes = Vec::new();
            if chunked {
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let size =
                        usize::from_str_radix(line.trim().split(';').next().unwrap(), 16).unwrap();
                    if size == 0 {
                        reader.read_line(&mut String::new()).unwrap();
                        break;
                    }
                    let mut part = vec![0; size];
                    reader.read_exact(&mut part).unwrap();
                    bytes.extend_from_slice(&part);
                    let mut ending = [0; 2];
                    reader.read_exact(&mut ending).unwrap();
                    assert_eq!(&ending, b"\r\n");
                }
            } else {
                bytes.resize(length.unwrap(), 0);
                reader.read_exact(&mut bytes).unwrap();
            }
            let request = serde_json::from_slice(&bytes).unwrap();
            write!(reader.get_mut(), "HTTP/1.1 {code} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            requests.push((first, request));
        }
        requests.pop().unwrap()
    });
    (format!("http://{address}/v1"), handle)
}

#[test]
fn provider_request_preserves_payload_and_parses_the_response() {
    let (url, server) = server(
        200,
        r#"{"choices":[{"message":{"content":"  Complete reply  "}}]}"#,
    );
    let messages = vec![serde_json::json!({"role":"user","content":"entire authored message"})];
    let config = config(url);
    assert_eq!(
        call_openai_chat(&config, messages.clone()).unwrap(),
        "Complete reply"
    );
    let (line, request) = server.join().unwrap();
    assert!(line.starts_with("POST /v1/chat/completions "));
    assert_eq!(request["messages"], serde_json::json!(messages));
    assert_eq!(request["model"], config.ai.model);
    assert_eq!(request["temperature"], 0);
}

#[test]
fn provider_context_limit_mapping_uses_status_and_real_error_body() {
    for (code, body, context_limit) in [
        (400, r#"{"error":{"code":"context_length_exceeded"}}"#, true),
        (413, "too large", true),
        (422, "Maximum context length exceeded", true),
        (500, "Maximum context length exceeded", false),
        (400, "invalid model synthetic-http-credential", false),
    ] {
        let (url, server) = server(code, body);
        let error = call_openai_chat(&config(url), vec![]).unwrap_err();
        assert_eq!(
            matches!(error, AiError::ContextLengthExceeded(_)),
            context_limit
        );
        assert!(!error.to_string().contains("synthetic-http-credential"));
        server.join().unwrap();
    }
}

#[test]
fn provider_rejects_malformed_and_empty_success_responses() {
    for body in [
        "invalid json",
        r#"{"choices":[{"message":{"content":" "}}]}"#,
    ] {
        let (url, server) = server(200, body);
        assert!(matches!(
            call_openai_chat(&config(url), vec![]),
            Err(AiError::Provider(_))
        ));
        server.join().unwrap();
    }
}

#[test]
fn mcp_request_preserves_jsonrpc_and_reports_protected_errors() {
    let (url, successful_server) = server(
        200,
        r#"{"jsonrpc":"2.0","id":1,"result":{"answer":"complete"}}"#,
    );
    let result = mcp_http_call(
        &url,
        "synthetic-http-credential",
        "tools/call",
        serde_json::json!({"name":"recall"}),
    )
    .unwrap();
    assert_eq!(result["result"]["answer"], "complete");
    let (_, request) = successful_server.join().unwrap();
    assert_eq!(request["method"], "tools/call");
    assert_eq!(request["params"]["name"], "recall");
    assert_eq!(request["jsonrpc"], "2.0");
    for (code, body) in [
        (
            403,
            "denied synthetic-http-credential private-mail-sentinel",
        ),
        (
            200,
            r#"{"error":{"message":"denied synthetic-http-credential private-mail-sentinel"}}"#,
        ),
    ] {
        let (url, server) = server(code, body);
        let error = mcp_http_call(&url, "synthetic-http-credential", "tools/call", Value::Null)
            .unwrap_err();
        assert!(matches!(error, AiError::Mcp(_)));
        assert!(error.to_string().contains("response body omitted"));
        assert!(!error.to_string().contains("private-mail-sentinel"));
        assert!(!error.to_string().contains("synthetic-http-credential"));
        server.join().unwrap();
    }
}
