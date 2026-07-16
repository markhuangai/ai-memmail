use super::*;

#[test]
fn reply_references_preserves_existing_refs_without_message_id() {
    let metadata = MessageMetadata {
        mailbox_id: "support".to_string(),
        uid_validity: 7,
        uid: 42,
        message_id: None,
        in_reply_to: Some("<m1@example.com>".to_string()),
        references: vec!["<root@example.com>".to_string()],
        from_addr: "a@example.com".to_string(),
        recipients: vec![],
        subject: "Hello".to_string(),
    };

    assert_eq!(
        reply_references(&metadata),
        vec!["<root@example.com>".to_string()]
    );
}

#[test]
fn message_ids_treats_invalid_blank_header_as_empty() {
    assert!(message_ids(" \t\r\n").is_empty());
}
