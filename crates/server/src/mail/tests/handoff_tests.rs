use super::*;

#[test]
fn handoff_composition_uses_latest_inbound_and_keeps_complete_transcript() {
    let mailbox = mailbox_config();
    let mut context = ThreadContext::empty("<root@example.com>".to_string());
    for (direction, id, subject, text) in [
        (
            MessageDirection::Inbound,
            "root",
            "Original",
            "First question",
        ),
        (
            MessageDirection::Inbound,
            "current",
            "Current question",
            "Current message",
        ),
        (
            MessageDirection::Outbound,
            "outbound",
            "Re: Current question",
            "Last response",
        ),
    ] {
        context.messages.push(ThreadMessage {
            direction,
            message_id: Some(format!("<{id}@example.com>")),
            in_reply_to: None,
            references: vec![
                "<root@example.com>".to_string(),
                "<current@example.com>".to_string(),
            ],
            from_addr: "person@example.com".to_string(),
            recipients: vec![mailbox.address.clone()],
            subject: subject.to_string(),
            authored_text: text.to_string(),
            body_truncated: false,
            timestamp: context.messages.len() as i64,
        });
    }
    let action = thread_handoff_action(
        &mailbox,
        &context,
        "human@example.com",
        "person@example.com",
    )
    .unwrap();
    assert_eq!(action.kind, OutboundActionKind::Forward);
    assert_eq!(action.subject, "Current question");
    assert_eq!(action.recipients, vec!["human@example.com"]);
    assert_eq!(action.reply_to.as_deref(), Some("person@example.com"));
    assert_eq!(action.in_reply_to.as_deref(), Some("<current@example.com>"));
    assert_eq!(
        action.references,
        vec!["<root@example.com>", "<current@example.com>"]
    );
    assert!(action.body.contains("First question"));
    assert!(action.body.contains("Current message"));
    assert!(action.body.contains("Last response"));
    assert!(action.message_id.unwrap().ends_with("@example.com>"));
    assert_eq!(action.html_body, None);

    context.messages[1].message_id = None;
    let action = thread_handoff_action(
        &mailbox,
        &context,
        "human@example.com",
        "person@example.com",
    )
    .unwrap();
    assert_eq!(action.in_reply_to, None);
    context
        .messages
        .iter_mut()
        .for_each(|message| message.direction = MessageDirection::Outbound);
    assert!(thread_handoff_action(
        &mailbox,
        &context,
        "human@example.com",
        "person@example.com"
    )
    .unwrap_err()
    .to_string()
    .contains("no inbound"));
}

#[test]
fn handoff_composition_rejects_incomplete_or_oversized_transcript() {
    let mailbox = mailbox_config();
    let mut context = ThreadContext::empty("root".to_string());
    assert!(thread_handoff_action(
        &mailbox,
        &context,
        "human@example.com",
        "person@example.com"
    )
    .is_err());
    context.messages.push(ThreadMessage {
        direction: MessageDirection::Inbound,
        message_id: Some("<current@example.com>".to_string()),
        in_reply_to: None,
        references: vec![],
        from_addr: "person@example.com".to_string(),
        recipients: vec![],
        subject: "Question".to_string(),
        authored_text: "Complete text".to_string(),
        body_truncated: true,
        timestamp: 0,
    });
    assert!(thread_handoff_action(
        &mailbox,
        &context,
        "human@example.com",
        "person@example.com"
    )
    .unwrap_err()
    .to_string()
    .contains("truncated"));
    context.messages[0].body_truncated = false;
    context.messages[0].authored_text = "x".repeat(5 * 1024 * 1024);
    assert!(thread_handoff_action(
        &mailbox,
        &context,
        "human@example.com",
        "person@example.com"
    )
    .unwrap_err()
    .to_string()
    .contains("5 MiB"));
}

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
    context.messages.push(ThreadMessage {
        direction: MessageDirection::Inbound,
        message_id: None,
        in_reply_to: None,
        references: vec![],
        from_addr: "person@example.com".to_string(),
        recipients: vec![],
        subject: "Re: Question".to_string(),
        authored_text: "Following up without headers.".to_string(),
        body_truncated: false,
        timestamp: 3,
    });

    let body = thread_handoff_body(&context).unwrap();
    assert!(body.contains("---------- Conversation handoff ---------"));
    assert!(body.contains("[1] Inbound"));
    assert!(body.contains("From: person@example.com"));
    assert!(body.contains("Can we talk?"));
    assert!(body.contains("[2] Outbound"));
    assert!(body.contains("Yes."));
    assert!(body.contains("[3] Inbound"));
    assert!(body.contains("To: (none)"));
    assert!(body.contains("Message-ID: (none)"));
    assert!(body.contains("References: (none)"));

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
