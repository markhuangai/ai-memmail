use crate::mail::{extract_authored_text, MessageDirection, ThreadContext, ThreadMessage};

#[test]
fn extracts_only_new_reply_text_when_canonical_history_exists() {
    let gmail_reply = "New answer.\n\nOn Mon, Jul 13, 2026 at 1:00 PM Mark\n<mark@example.com> wrote:\n> Original question";
    let extracted = extract_authored_text(gmail_reply, true);

    assert_eq!(extracted.authored_text, "New answer.");
    assert!(extracted.quoted_text_removed);
    assert_eq!(
        extract_authored_text(gmail_reply, false).authored_text,
        gmail_reply
    );
}

#[test]
fn preserves_body_when_quote_marker_has_no_authored_prefix() {
    let body = "> inline response\n> original text";
    let extracted = extract_authored_text(body, true);

    assert_eq!(extracted.authored_text, body);
    assert!(!extracted.quoted_text_removed);
}

#[test]
fn preserves_unquoted_reply_with_canonical_history() {
    let body = "A fresh reply without quoted history.";
    let extracted = extract_authored_text(body, true);

    assert_eq!(extracted.authored_text, body);
    assert!(!extracted.quoted_text_removed);
}

#[test]
fn thread_context_reports_truncated_history() {
    let mut context = ThreadContext::empty("<root@example.com>".to_string());
    assert_eq!(context.thread_id, "<root@example.com>");
    assert!(!context.has_truncated_body());

    context.messages.push(ThreadMessage {
        direction: MessageDirection::Outbound,
        message_id: Some("<sent@example.com>".to_string()),
        in_reply_to: None,
        references: vec![],
        from_addr: "support@example.com".to_string(),
        recipients: vec!["person@example.com".to_string()],
        subject: "Question".to_string(),
        authored_text: "Original message".to_string(),
        body_truncated: true,
        timestamp: 1_700_000_000,
    });
    assert!(context.has_truncated_body());
}
