use super::*;
use crate::logging::test_capture::capture;

#[test]
fn provider_context_limit_logs_cause_and_forward_outcome_at_info() {
    for fail_send in [false, true] {
        let (url, server) = crate::ai_external::http_tests::server(
            413,
            "context_length_exceeded synthetic-http-credential",
        );
        let mut config = config();
        config.ai.api_url = url;
        config.ai.api_secret = "synthetic-http-credential".to_string();
        config.prompts.root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
        config.prompts.safety_scan = "safety-scan.md".into();
        let mail = if fail_send {
            FakeMail::new(vec![]).with_fail_send()
        } else {
            FakeMail::new(vec![])
        };
        let message = inbound(
            71,
            "person@example.com",
            "Question",
            "private body omitted from logs",
        );
        let processing = MemoryProcessingStore::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_, events) = capture(|| {
            runtime.block_on(process_message(
                &config,
                &config.mailboxes[0],
                &StdoutLogger,
                "offline-provider-limit",
                &mail,
                &LiveDecisionEngine::default(),
                &processing,
                message,
            ))
        });
        let text = serde_json::to_string(&events).unwrap();
        assert!(text.contains("offline-provider-limit"));
        assert!(text.contains("context_limit"));
        assert!(text.contains("provider rejected"));
        assert!(text.contains("smtp_send"));
        assert!(text.contains(if fail_send {
            "send failed"
        } else {
            "forwarded"
        }));
        assert!(!text.contains("synthetic-http-credential"));
        assert!(!text.contains("private body omitted from logs"));
        assert!(events.iter().all(|event| event["level"] != "DEBUG"));
        if fail_send {
            assert!(events.iter().any(|event| event["level"] == "ERROR"));
        } else {
            assert_eq!(mail.sent().len(), 1);
            assert_eq!(mail.sent()[0].recipients, vec!["human@example.com"]);
        }
        server.join().unwrap();
    }
}

#[test]
fn handoff_logs_rejection_and_delivery_outcome_without_transcript() {
    let config = config();
    let mailbox = &config.mailboxes[0];
    let message = inbound(
        72,
        "person@example.com",
        "Question",
        "private current message",
    );
    let handoff = crate::storage::ThreadHandoff {
        mailbox_id: mailbox.id.clone(),
        thread_id: "thread-72".to_string(),
        destination: "human@example.com".to_string(),
        remote_target: "person@example.com".to_string(),
        state: "active".to_string(),
        last_error: None,
        updated_at: "2026-11-10".to_string(),
    };
    for (truncated, fail_send) in [(true, false), (false, false), (false, true)] {
        let context = ThreadContext {
            thread_id: handoff.thread_id.clone(),
            messages: vec![ThreadMessage {
                direction: MessageDirection::Inbound,
                message_id: Some("<prior@example.com>".to_string()),
                in_reply_to: None,
                references: vec![],
                from_addr: "person@example.com".to_string(),
                recipients: vec![mailbox.address.clone()],
                subject: "Earlier question".to_string(),
                authored_text: "private transcript".to_string(),
                body_truncated: truncated,
                timestamp: 1,
            }],
        };
        let mail = if fail_send {
            FakeMail::new(vec![]).with_fail_send()
        } else {
            FakeMail::new(vec![])
        };
        let processing = MemoryProcessingStore::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_, events) = capture(|| {
            runtime.block_on(route_active_thread_handoff(
                mailbox,
                &StdoutLogger,
                "offline-handoff",
                &mail,
                &processing,
                &message,
                &context,
                &handoff,
            ))
        });
        let text = serde_json::to_string(&events).unwrap();
        assert!(text.contains("offline-handoff"));
        assert!(text.contains("thread_handoff"));
        assert!(text.contains("started"));
        assert!(!text.contains("private transcript"));
        assert!(!text.contains("private current message"));
        if truncated {
            assert!(text.contains("truncated stored message bodies"));
            assert!(text.contains("forwarded"));
        } else if fail_send {
            assert!(text.contains("failed"));
            assert!(text.contains("send failed"));
        } else {
            assert!(text.contains("sent"));
            let action = &mail.sent()[0];
            assert!(action.body.contains("private current message"));
            assert!(action.body.contains("private transcript"));
            assert_eq!(action.in_reply_to, message.metadata.message_id);
        }
    }
}

#[test]
fn mark_seen_logs_start_and_real_store_outcome_with_safe_settings() {
    for rejected in [false, true] {
        let mut steps = vec![
            ("LOGIN ", "$TAG OK authenticated\r\n".to_string()),
            (
                "SELECT ",
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 1] valid\r\n$TAG OK selected\r\n".to_string(),
            ),
            (
                "UID STORE 7 ",
                if rejected {
                    "* 1 FETCH (UID 7 FLAGS (\\Seen))\r\n$TAG NO store denied synthetic-imap-password\r\n".to_string()
                } else {
                    "* 1 FETCH (UID 7 FLAGS (\\Seen))\r\n$TAG OK stored\r\n".to_string()
                },
            ),
        ];
        if !rejected {
            steps.push(("LOGOUT", "* BYE done\r\n$TAG OK logout\r\n".to_string()));
        }
        let (address, server) =
            crate::mail_external::protocol_tests::server("* OK ready\r\n", steps);
        let mut config = config();
        let mailbox = &mut config.mailboxes[0];
        mailbox.imap.host = address.ip().to_string();
        mailbox.imap.port = address.port();
        mailbox.imap.tls = false;
        mailbox.imap.password = "synthetic-imap-password".to_string();
        let message = inbound(7, "person@example.com", "Question", "private body");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_, events) = capture(|| {
            runtime.block_on(mark_seen(
                mailbox,
                &StdoutLogger,
                "offline-imap-seen",
                &LiveMailTransport::default(),
                &message,
                "seen",
            ))
        });
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["level"], "INFO");
        let start = events[0]["fields"]["event"].as_str().unwrap();
        for required in [
            "offline-imap-seen",
            "support",
            "imap_mark_seen",
            "started",
            "folder=INBOX",
            "tls=false",
            "port=",
            "message_uid: Some(7)",
        ] {
            assert!(
                start.contains(required),
                "missing diagnostic field: {required}"
            );
        }
        let outcome = events[1]["fields"]["event"].as_str().unwrap();
        for required in [
            "offline-imap-seen",
            "support",
            "imap_mark_seen",
            "duration_ms",
            "message_uid: Some(7)",
        ] {
            assert!(
                outcome.contains(required),
                "missing diagnostic field: {required}"
            );
        }
        if rejected {
            assert_eq!(events[1]["level"], "ERROR");
            assert!(outcome.contains("failed"));
            assert!(outcome.contains("store denied"));
        } else {
            assert_eq!(events[1]["level"], "INFO");
            assert!(outcome.contains("seen"));
        }
        let text = serde_json::to_string(&events).unwrap();
        assert!(!text.contains("synthetic-imap-password"));
        assert!(!text.contains("private body"));
        server.join().unwrap();
    }
}

#[test]
fn provider_failures_log_status_and_safe_cause_without_echoed_mail() {
    for (code, body, cause) in [
        (
            400,
            r#"{"error":{"code":"model_not_found","message":"private-mail-sentinel synthetic-http-credential"}}"#,
            "model not found",
        ),
        (
            500,
            "private-mail-sentinel synthetic-http-credential",
            "Internal Server Error",
        ),
    ] {
        let (url, server) = crate::ai_external::http_tests::server(code, body);
        let mut config = config();
        config.ai.api_url = url;
        config.ai.api_secret = "synthetic-http-credential".to_string();
        config.prompts.root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
        config.prompts.safety_scan = "safety-scan.md".into();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_, events) = capture(|| {
            runtime.block_on(process_message(
                &config,
                &config.mailboxes[0],
                &StdoutLogger,
                "offline-provider-error",
                &FakeMail::new(vec![]),
                &LiveDecisionEngine::default(),
                &MemoryProcessingStore::default(),
                inbound(
                    73,
                    "person@example.com",
                    "Question",
                    "private-mail-sentinel",
                ),
            ))
        });
        let text = serde_json::to_string(&events).unwrap();
        for required in [
            "offline-provider-error",
            "safety_scan",
            "failed",
            &format!("HTTP {code}"),
            cause,
            "response body omitted",
        ] {
            assert!(
                text.contains(required),
                "missing safe diagnostic: {required}"
            );
        }
        assert!(events.iter().any(|event| event["level"] == "ERROR"));
        assert!(!text.contains("private-mail-sentinel"));
        assert!(!text.contains("synthetic-http-credential"));
        server.join().unwrap();
    }
}

#[test]
fn mcp_failures_log_safe_jsonrpc_cause_without_echoed_mail() {
    let (url, server) = crate::ai_external::http_tests::server_for_calls(
        200,
        r#"{"error":{"code":-32603,"message":"private-mail-sentinel synthetic-http-credential"}}"#,
        2,
    );
    let mut config = config();
    config.prompts.root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    config.mailboxes[0].agent.system_prompt_path = "support-agent.md".into();
    config.mailboxes[0].mcp_servers = vec!["offline".to_string()];
    config.mcp_servers.insert(
        "offline".to_string(),
        crate::config::McpServerConfig {
            transport: crate::config::McpTransport::StreamableHttp,
            command: None,
            args: vec![],
            env: BTreeMap::from([(
                "DENSE_MEM_API_KEY".to_string(),
                "synthetic-http-credential".to_string(),
            )]),
            url: Some(url),
        },
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (result, events) = capture(|| {
        runtime.block_on(agent_decision_or_retry(
            &config,
            &config.mailboxes[0],
            &StdoutLogger,
            "offline-mcp-error",
            &LiveDecisionEngine::default(),
            &MemoryProcessingStore::default(),
            &FakeMail::new(vec![]),
            &inbound(
                74,
                "person@example.com",
                "Question",
                "private-mail-sentinel",
            ),
            &ThreadContext::empty("thread-74".to_string()),
            Instant::now(),
        ))
    });
    assert_eq!(result, None);
    let text = serde_json::to_string(&events).unwrap();
    for required in [
        "offline-mcp-error",
        "agent_decision",
        "failed",
        "MCP JSON-RPC error",
        "internal error",
        "response body omitted",
    ] {
        assert!(
            text.contains(required),
            "missing safe diagnostic: {required}"
        );
    }
    assert!(events.iter().any(|event| event["level"] == "ERROR"));
    assert!(!text.contains("private-mail-sentinel"));
    assert!(!text.contains("synthetic-http-credential"));
    let (_, request) = server.join().unwrap();
    assert_eq!(request["params"]["name"], "recall_memory");
}
