use crate::config::{AcceptedCondition, AgentConfig, ImapConfig, MailboxConfig, SmtpConfig};
use crate::mail::{ComposedEmail, OutboundAction, OutboundActionKind};

use super::*;

#[test]
fn recipient_prefilter_queries_use_common_delivery_headers() {
    let mut mailbox = mailbox_config();
    mailbox.accepted_conditions = vec![AcceptedCondition {
        recipients: vec!["Support@example.com".to_string()],
        subject_regex: vec!["(?i)billing".to_string()],
    }];

    let queries = recipient_prefilter_queries(&mailbox).unwrap();

    assert_eq!(
        queries,
        vec![
            "UNSEEN HEADER To \"support@example.com\"".to_string(),
            "UNSEEN HEADER Cc \"support@example.com\"".to_string(),
            "UNSEEN HEADER Delivered-To \"support@example.com\"".to_string(),
            "UNSEEN HEADER X-Original-To \"support@example.com\"".to_string(),
            "UNSEEN HEADER Envelope-To \"support@example.com\"".to_string(),
        ]
    );
}

#[test]
fn subject_only_conditions_do_not_build_recipient_prefilter_queries() {
    let mut mailbox = mailbox_config();
    mailbox.accepted_conditions = vec![AcceptedCondition {
        recipients: vec![],
        subject_regex: vec!["urgent".to_string()],
    }];

    assert_eq!(recipient_prefilter_queries(&mailbox), None);
}

#[test]
fn sent_uid_query_uses_non_reversing_upper_bound() {
    assert_eq!(
        sent_uid_query(42, 0).unwrap().as_deref(),
        Some("UID 43:4294967295 SINCE 01-Jan-1970")
    );
    assert_eq!(sent_uid_query(u32::MAX as u64, 0).unwrap(), None);
}

#[test]
fn sent_sync_range_restarts_when_backfill_expands() {
    let cursor = SentSyncCursor {
        folder_name: "Sent".to_string(),
        uid_validity: 7,
        last_uid: 900,
        backfill_cutoff: 2_000,
    };

    assert_eq!(
        sent_sync_range(Some(&cursor), "Sent", 7, 2_100),
        (900, 2_000)
    );
    assert_eq!(sent_sync_range(Some(&cursor), "Sent", 7, 1_000), (0, 1_000));
}

#[test]
fn build_message_sets_reply_thread_headers() {
    let smtp = SmtpConfig {
        host: "smtp.example.com".to_string(),
        port: 587,
        starttls: true,
        username: "support@example.com".to_string(),
        password: "secret".to_string(),
        from: "support@example.com".to_string(),
    };
    let action = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Question".to_string(),
        body: "Answer".to_string(),
        html_body: None,
        reason: "known answer".to_string(),
        reply_to: Some("remote@example.com".to_string()),
        message_id: Some("<reply@example.com>".to_string()),
        in_reply_to: Some("<inbound@example.com>".to_string()),
        references: vec![
            "<root@example.com>".to_string(),
            "<inbound@example.com>".to_string(),
        ],
    };

    let message = build_message(&smtp, &action).unwrap();
    let rendered = String::from_utf8(message.formatted()).unwrap();

    assert!(rendered.contains("Message-ID: <reply@example.com>\r\n"));
    assert!(rendered.contains("Reply-To: remote@example.com\r\n"));
    assert!(rendered.contains("In-Reply-To: <inbound@example.com>\r\n"));
    assert!(rendered.contains("References: <root@example.com> <inbound@example.com>\r\n"));
}

#[test]
fn build_message_uses_html_body_as_single_html_part() {
    let smtp = SmtpConfig {
        host: "smtp.example.com".to_string(),
        port: 587,
        starttls: true,
        username: "support@example.com".to_string(),
        password: "secret".to_string(),
        from: "support@example.com".to_string(),
    };
    let action = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Question".to_string(),
        body: "Answer".to_string(),
        html_body: Some("<p>Answer</p>".to_string()),
        reason: "known answer".to_string(),
        reply_to: None,
        message_id: Some("<reply@example.com>".to_string()),
        in_reply_to: Some("<inbound@example.com>".to_string()),
        references: vec![],
    };

    let message = build_message(&smtp, &action).unwrap();
    let rendered = String::from_utf8(message.formatted()).unwrap();

    assert!(rendered.contains("Content-Type: text/html; charset=utf-8\r\n"));
    assert!(rendered.contains("<p>Answer</p>"));
    assert!(!rendered.contains("multipart/alternative"));
    assert!(!rendered.contains("Content-Type: text/plain"));
}

#[test]
fn build_composed_message_uses_multipart_and_hides_bcc_header() {
    let smtp = SmtpConfig {
        host: "smtp.example.com".to_string(),
        port: 587,
        starttls: true,
        username: "support@example.com".to_string(),
        password: "secret".to_string(),
        from: "support@example.com".to_string(),
    };
    let message = ComposedEmail {
        to: vec!["person@example.com".to_string()],
        cc: vec!["copy@example.com".to_string()],
        bcc: vec!["hidden@example.com".to_string()],
        subject: "Re: Question".to_string(),
        text_body: "Answer".to_string(),
        html_body: Some("<p>Answer</p>".to_string()),
        message_id: Some("<reply@example.com>".to_string()),
        in_reply_to: Some("<inbound@example.com>".to_string()),
        references: vec!["<inbound@example.com>".to_string()],
    };

    let rendered =
        String::from_utf8(build_composed_message(&smtp, &message).unwrap().formatted()).unwrap();

    assert!(rendered.contains("Content-Type: multipart/alternative;"));
    assert!(rendered.contains("To: person@example.com\r\n"));
    assert!(rendered.contains("Cc: copy@example.com\r\n"));
    assert!(!rendered.contains("Bcc: hidden@example.com"));
    assert!(rendered.contains("Content-Type: text/plain; charset=utf-8\r\n"));
    assert!(rendered.contains("Content-Type: text/html; charset=utf-8\r\n"));
}

fn mailbox_config() -> MailboxConfig {
    MailboxConfig {
        id: "support".to_string(),
        address: "support@example.com".to_string(),
        enabled: true,
        poll_interval_seconds: 30,
        safety_forward_to: vec!["safety@example.com".to_string()],
        signature: None,
        accepted_conditions: vec![],
        mcp_servers: vec![],
        agent: AgentConfig {
            system_prompt_path: "agent.md".into(),
            default_forward_to: vec!["human@example.com".to_string()],
        },
        imap: ImapConfig {
            host: "imap.example.com".to_string(),
            port: 993,
            tls: true,
            username: "support@example.com".to_string(),
            password: "secret".to_string(),
            folder: "INBOX".to_string(),
            sent_folder: None,
            sent_backfill_days: 0,
        },
        smtp: SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            starttls: true,
            username: "support@example.com".to_string(),
            password: "secret".to_string(),
            from: "support@example.com".to_string(),
        },
    }
}
