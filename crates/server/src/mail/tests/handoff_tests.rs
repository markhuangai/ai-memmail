use super::*;

#[test]
fn thread_handoff_body_formats_chain_and_rejects_truncated_messages() {
    let mut context = ThreadContext::empty("<root@example.com>".to_string());
    context.messages.push(ThreadMessage {
        direction: MessageDirection::Inbound,
        message_id: Some("<root@example.com>".to_string()),
        in_reply_to: None,
        references: vec![],
        from_addr: "person@example.com".to_string(),
        recipients: vec!["support@example.com".to_string()],
        subject: "Question".to_string(),
        authored_text: "Can we talk?".to_string(),
        body_truncated: false,
        timestamp: 1,
    });
    context.messages.push(ThreadMessage {
        direction: MessageDirection::Outbound,
        message_id: Some("<reply@example.com>".to_string()),
        in_reply_to: Some("<root@example.com>".to_string()),
        references: vec!["<root@example.com>".to_string()],
        from_addr: "support@example.com".to_string(),
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Question".to_string(),
        authored_text: "Yes.".to_string(),
        body_truncated: false,
        timestamp: 2,
    });

    let body = thread_handoff_body(&context).unwrap();
    assert!(body.contains("---------- Conversation handoff ---------"));
    assert!(body.contains("[1] Inbound"));
    assert!(body.contains("From: person@example.com"));
    assert!(body.contains("Can we talk?"));
    assert!(body.contains("[2] Outbound"));
    assert!(body.contains("Yes."));

    context.messages[1].body_truncated = true;
    let error = thread_handoff_body(&context).unwrap_err();
    assert!(error.to_string().contains("truncated"));
}

#[test]
fn thread_handoff_body_rejects_empty_or_oversized_chain() {
    let empty = ThreadContext::empty("<root@example.com>".to_string());
    let error = thread_handoff_body(&empty).unwrap_err();
    assert!(error.to_string().contains("at least one stored message"));

    let mut oversized = ThreadContext::empty("<root@example.com>".to_string());
    oversized.messages.push(ThreadMessage {
        direction: MessageDirection::Inbound,
        message_id: Some("<root@example.com>".to_string()),
        in_reply_to: None,
        references: vec![],
        from_addr: "person@example.com".to_string(),
        recipients: vec!["support@example.com".to_string()],
        subject: "Question".to_string(),
        authored_text: "x".repeat(5 * 1024 * 1024),
        body_truncated: false,
        timestamp: 1,
    });

    let error = thread_handoff_body(&oversized).unwrap_err();
    assert!(error.to_string().contains("exceeds 5 MiB"));
}
