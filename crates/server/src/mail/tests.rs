use std::sync::{Arc, Mutex};

use crate::config::{AcceptedCondition, AgentConfig, ImapConfig, MailboxConfig, SmtpConfig};

use super::*;

mod thread_context_tests;

#[derive(Clone, Default)]
struct FakeBlockingMailClient {
    state: Arc<Mutex<FakeBlockingMailState>>,
}

#[derive(Default)]
struct FakeBlockingMailState {
    fetched_limits: Vec<usize>,
    sent_limits: Vec<usize>,
    sent_subjects: Vec<String>,
    seen_uids: Vec<u64>,
}

impl BlockingMailClient for FakeBlockingMailClient {
    fn fetch_unseen(
        &self,
        mailbox: &MailboxConfig,
        limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError> {
        self.state
            .lock()
            .expect("fake mail state")
            .fetched_limits
            .push(limit);
        Ok(vec![InboundMessage {
            metadata: MessageMetadata {
                mailbox_id: mailbox.id.clone(),
                uid_validity: 9,
                uid: 42,
                message_id: Some("<m1@example.com>".to_string()),
                in_reply_to: None,
                references: vec![],
                from_addr: "sender@example.com".to_string(),
                recipients: vec![],
                subject: "Question".to_string(),
            },
            plain_text: "Body".to_string(),
        }])
    }

    fn send(&self, _smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
        self.state
            .lock()
            .expect("fake mail state")
            .sent_subjects
            .push(action.subject.clone());
        Ok(())
    }

    fn mark_seen(&self, _mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
        self.state
            .lock()
            .expect("fake mail state")
            .seen_uids
            .push(uid);
        Ok(())
    }

    fn fetch_sent(
        &self,
        mailbox: &MailboxConfig,
        _cursor: Option<&SentSyncCursor>,
        _backfill_cutoff: i64,
        limit: usize,
    ) -> Result<SentFetchBatch, MailError> {
        self.state
            .lock()
            .expect("fake mail state")
            .sent_limits
            .push(limit);
        Ok(SentFetchBatch {
            folder_name: "Sent".to_string(),
            uid_validity: 10,
            messages: vec![SentMessage {
                message: InboundMessage {
                    metadata: MessageMetadata {
                        mailbox_id: mailbox.id.clone(),
                        uid_validity: 10,
                        uid: 43,
                        message_id: Some("<sent@example.com>".to_string()),
                        in_reply_to: None,
                        references: vec![],
                        from_addr: mailbox.address.clone(),
                        recipients: vec!["person@example.com".to_string()],
                        subject: "Sent question".to_string(),
                    },
                    plain_text: "Original".to_string(),
                },
                internal_date: Some(1_700_000_000),
            }],
            complete: true,
        })
    }
}

#[derive(Clone)]
struct SentUnavailableBlockingClient;

impl BlockingMailClient for SentUnavailableBlockingClient {
    fn fetch_unseen(
        &self,
        _mailbox: &MailboxConfig,
        _limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError> {
        Ok(vec![])
    }

    fn send(&self, _smtp: &SmtpConfig, _action: &OutboundAction) -> Result<(), MailError> {
        Ok(())
    }

    fn mark_seen(&self, _mailbox: &MailboxConfig, _uid: u64) -> Result<(), MailError> {
        Ok(())
    }
}

struct SentUnavailableTransport;

#[async_trait::async_trait]
impl MailTransport for SentUnavailableTransport {
    async fn fetch_unseen(
        &self,
        _mailbox: &MailboxConfig,
        _limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError> {
        Ok(vec![])
    }

    async fn send(&self, _smtp: &SmtpConfig, _action: &OutboundAction) -> Result<(), MailError> {
        Ok(())
    }

    async fn mark_seen(&self, _mailbox: &MailboxConfig, _uid: u64) -> Result<(), MailError> {
        Ok(())
    }
}

#[tokio::test]
async fn sent_sync_trait_defaults_fail_closed() {
    let mailbox = mailbox_config();
    let transport_error = SentUnavailableTransport
        .fetch_sent(&mailbox, None, 0, 10)
        .await
        .unwrap_err();
    let client_error = SentUnavailableBlockingClient
        .fetch_sent(&mailbox, None, 0, 10)
        .unwrap_err();

    assert!(transport_error.to_string().contains("unavailable"));
    assert!(client_error.to_string().contains("unavailable"));
}

#[tokio::test]
async fn live_transport_delegates_to_blocking_client() {
    let client = FakeBlockingMailClient::default();
    let transport = LiveMailTransport::new(client.clone());
    let _default_transport = LiveMailTransport::default();
    let mailbox = mailbox_config();
    let action = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Question".to_string(),
        body: "Answer".to_string(),
        reason: "test".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };

    let fetched = transport.fetch_unseen(&mailbox, 3).await.unwrap();
    let sent = transport.fetch_sent(&mailbox, None, 0, 4).await.unwrap();
    transport.send(&mailbox.smtp, &action).await.unwrap();
    transport.mark_seen(&mailbox, 42).await.unwrap();

    assert_eq!(fetched[0].metadata.mailbox_id, "support");
    assert_eq!(sent.messages[0].message.metadata.uid, 43);
    let state = client.state.lock().expect("fake mail state");
    assert_eq!(state.fetched_limits, vec![3]);
    assert_eq!(state.sent_limits, vec![4]);
    assert_eq!(state.sent_subjects, vec!["Re: Question"]);
    assert_eq!(state.seen_uids, vec![42]);
}

#[test]
fn mail_error_messages_include_context() {
    assert_eq!(
        MailError::Task("join failed".to_string()).to_string(),
        "mail task failed: join failed"
    );
    assert_eq!(
        MailError::Imap("login failed".to_string()).to_string(),
        "imap error: login failed"
    );
    assert_eq!(
        MailError::Smtp("send failed".to_string()).to_string(),
        "smtp error: send failed"
    );
    assert_eq!(
        MailError::Parse("invalid body".to_string()).to_string(),
        "message parse error: invalid body"
    );
    assert_eq!(
        MailError::Build("bad header".to_string()).to_string(),
        "message build error: bad header"
    );
}

#[test]
fn metadata_builds_stable_dedupe_key() {
    let metadata = MessageMetadata {
        mailbox_id: "support".to_string(),
        uid_validity: 7,
        uid: 42,
        message_id: None,
        in_reply_to: None,
        references: vec![],
        from_addr: "a@example.com".to_string(),
        recipients: vec![],
        subject: "Hello".to_string(),
    };
    assert_eq!(
        metadata.dedupe_key(),
        DedupeKey {
            mailbox_id: "support".to_string(),
            uid_validity: 7,
            uid: 42
        }
    );
}

#[test]
fn thread_id_falls_back_through_reply_headers_and_uid() {
    let mut metadata = MessageMetadata {
        mailbox_id: "support".to_string(),
        uid_validity: 7,
        uid: 42,
        message_id: None,
        in_reply_to: None,
        references: vec![],
        from_addr: "a@example.com".to_string(),
        recipients: vec![],
        subject: "Hello".to_string(),
    };
    assert_eq!(metadata.thread_id(), "support:7:42");

    metadata.message_id = Some("<message@example.com>".to_string());
    assert_eq!(metadata.thread_id(), "<message@example.com>");

    metadata.in_reply_to = Some("<reply@example.com>".to_string());
    assert_eq!(metadata.thread_id(), "<reply@example.com>");

    metadata.references = vec!["<root@example.com>".to_string()];
    assert_eq!(metadata.thread_id(), "<root@example.com>");
}

#[test]
fn validates_reply_requires_recipient_subject_and_body() {
    let action = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec![],
        subject: "".to_string(),
        body: "".to_string(),
        reason: "test".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    let errors = validate_outbound_action(&action).unwrap_err();
    assert_eq!(errors.len(), 3);
}

#[test]
fn validates_noop_has_no_recipients() {
    let action = OutboundAction {
        kind: OutboundActionKind::Noop,
        recipients: vec!["person@example.com".to_string()],
        subject: "".to_string(),
        body: "".to_string(),
        reason: "nothing to do".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    let errors = validate_outbound_action(&action).unwrap_err();
    assert_eq!(errors[0].field, "recipients");
}

#[test]
fn validates_complete_reply_forward_and_noop_actions() {
    let reply = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Hello".to_string(),
        body: "Thanks".to_string(),
        reason: "known answer".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    assert!(validate_outbound_action(&reply).is_ok());

    let forward = OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: vec!["human@example.com".to_string()],
        subject: "Review".to_string(),
        body: "Please review".to_string(),
        reason: "needs human review".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    assert!(validate_outbound_action(&forward).is_ok());

    let noop = OutboundAction {
        kind: OutboundActionKind::Noop,
        recipients: vec![],
        subject: "".to_string(),
        body: "".to_string(),
        reason: "nothing safe to do".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    assert!(validate_outbound_action(&noop).is_ok());
}

#[test]
fn parses_simple_inbound_message() {
    let raw = b"From: Sender <sender@example.com>\r\nTo: Support <support@example.com>\r\nSubject: Hello\r\nMessage-ID: <m1@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody text";
    let message = parse_inbound_message("support", 9, 10, raw).unwrap();
    assert_eq!(message.metadata.mailbox_id, "support");
    assert_eq!(message.metadata.uid_validity, 9);
    assert_eq!(message.metadata.uid, 10);
    assert_eq!(
        message.metadata.message_id,
        Some("<m1@example.com>".to_string())
    );
    assert_eq!(
        message.metadata.recipients,
        vec!["support@example.com".to_string()]
    );
    assert_eq!(message.metadata.subject, "Hello");
    assert_eq!(message.plain_text, "Body text");
}

#[test]
fn parses_common_recipient_and_delivery_headers() {
    let raw = b"From: Sender <sender@example.com>\r\nTo: Support <support@example.com>\r\nCc: Ops: Ops One <ops1@example.com>, ops2@example.com;\r\nDelivered-To: routed@example.com\r\nX-Original-To: Original <original@example.com>\r\nEnvelope-To: envelope@example.com\r\nSubject: Hello\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody text";
    let message = parse_inbound_message("support", 9, 10, raw).unwrap();

    assert_eq!(
        message.metadata.recipients,
        vec![
            "support@example.com".to_string(),
            "ops1@example.com".to_string(),
            "ops2@example.com".to_string(),
            "routed@example.com".to_string(),
            "original@example.com".to_string(),
            "envelope@example.com".to_string(),
        ]
    );
}

#[test]
fn accepted_conditions_match_group_semantics() {
    let message = filter_message(
        "Billing escalation",
        vec![
            "Support@example.com".to_string(),
            "team@example.com".to_string(),
        ],
    );

    assert!(message_matches_accepted_conditions(&message, &[]));
    assert!(message_matches_accepted_conditions(
        &message,
        &[AcceptedCondition {
            recipients: vec!["support@example.com".to_string()],
            subject_regex: vec!["(?i)billing".to_string()],
        }]
    ));
    assert!(message_matches_accepted_conditions(
        &message,
        &[AcceptedCondition {
            recipients: vec!["support@example.com".to_string()],
            subject_regex: vec![],
        }]
    ));
    assert!(!message_matches_accepted_conditions(
        &message,
        &[AcceptedCondition {
            recipients: vec!["support@example.com".to_string()],
            subject_regex: vec!["sales".to_string()],
        }]
    ));
    assert!(message_matches_accepted_conditions(
        &message,
        &[
            AcceptedCondition {
                recipients: vec!["other@example.com".to_string()],
                subject_regex: vec!["sales".to_string()],
            },
            AcceptedCondition {
                recipients: vec![],
                subject_regex: vec!["escalation".to_string()],
            },
        ]
    ));
}

#[test]
fn accepted_condition_recipient_prefilter_requires_recipients_in_every_group() {
    let recipient_group = AcceptedCondition {
        recipients: vec!["Support@example.com".to_string()],
        subject_regex: vec!["(?i)billing".to_string()],
    };
    let subject_only_group = AcceptedCondition {
        recipients: vec![],
        subject_regex: vec!["urgent".to_string()],
    };

    assert!(!accepted_conditions_can_prefilter_by_recipient(&[]));
    assert!(accepted_conditions_can_prefilter_by_recipient(&[
        recipient_group.clone()
    ]));
    assert!(!accepted_conditions_can_prefilter_by_recipient(&[
        recipient_group,
        subject_only_group,
    ]));
    assert_eq!(
        accepted_condition_recipient_filter_values(&[
            AcceptedCondition {
                recipients: vec!["Support@example.com".to_string()],
                subject_regex: vec![],
            },
            AcceptedCondition {
                recipients: vec![
                    "support@example.com".to_string(),
                    "ops@example.com".to_string()
                ],
                subject_regex: vec![],
            },
        ]),
        vec![
            "support@example.com".to_string(),
            "ops@example.com".to_string()
        ]
    );
    assert_eq!(
        normalize_email_address("Operations <OPS@example.com>"),
        Some("ops@example.com".to_string())
    );
    assert_eq!(normalize_email_address("not-an-address"), None);
}

#[test]
fn parses_thread_headers_and_derives_thread_id() {
    let raw = b"From: Sender <sender@example.com>\r\nSubject: Follow up\r\nMessage-ID: <m2@example.com>\r\nIn-Reply-To: <m1@example.com>\r\nReferences: <root@example.com> <m1@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nFollow up";
    let message = parse_inbound_message("support", 9, 11, raw).unwrap();

    assert_eq!(
        message.metadata.message_id,
        Some("<m2@example.com>".to_string())
    );
    assert_eq!(
        message.metadata.in_reply_to,
        Some("<m1@example.com>".to_string())
    );
    assert_eq!(
        message.metadata.references,
        vec![
            "<root@example.com>".to_string(),
            "<m1@example.com>".to_string()
        ]
    );
    assert_eq!(message.metadata.thread_id(), "<root@example.com>");
}

#[test]
fn parse_inbound_message_keeps_unparseable_message_id_headers() {
    let raw = b"From: Sender <sender@example.com>\r\nSubject: Bad IDs\r\nMessage-ID: not-a-message-id\r\nReferences: also-not-a-message-id\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody";
    let message = parse_inbound_message("support", 9, 12, raw).unwrap();

    assert_eq!(
        message.metadata.message_id,
        Some("not-a-message-id".to_string())
    );
    assert_eq!(
        message.metadata.references,
        vec!["also-not-a-message-id".to_string()]
    );
    assert!(message_ids("").is_empty());
}

#[test]
fn reply_references_append_inbound_message_id_once() {
    let mut metadata = MessageMetadata {
        mailbox_id: "support".to_string(),
        uid_validity: 7,
        uid: 42,
        message_id: Some("<m2@example.com>".to_string()),
        in_reply_to: Some("<m1@example.com>".to_string()),
        references: vec!["<root@example.com>".to_string()],
        from_addr: "a@example.com".to_string(),
        recipients: vec![],
        subject: "Hello".to_string(),
    };
    assert_eq!(
        reply_references(&metadata),
        vec![
            "<root@example.com>".to_string(),
            "<m2@example.com>".to_string()
        ]
    );

    metadata.references.push("<m2@example.com>".to_string());
    assert_eq!(
        reply_references(&metadata),
        vec![
            "<root@example.com>".to_string(),
            "<m2@example.com>".to_string()
        ]
    );
}

#[test]
fn reply_recipient_extracts_address_from_display_header() {
    assert_eq!(
        reply_recipient("Josh <joshua@example.com>"),
        "joshua@example.com"
    );
    assert_eq!(reply_recipient("plain@example.com"), "plain@example.com");
    assert_eq!(reply_recipient("  not an address  "), "not an address");
}

#[test]
fn automated_reply_body_appends_escalation_notice_once() {
    let body = automated_reply_body("Answer");

    assert_eq!(
        body,
        "Answer\n\n--\nThis automated reply was sent on Mark's behalf. If this needs Mark's attention, reply with: escalation to human"
    );
    assert_eq!(automated_reply_body(&body), body);
}

#[test]
fn parses_text_part_from_multipart_message() {
    let raw = b"From: sender@example.com\r\nSubject: Multipart\r\nContent-Type: multipart/alternative; boundary=abc\r\n\r\n--abc\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n--abc\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML body</p>\r\n--abc--";
    let message = parse_inbound_message("support", 1, 2, raw).unwrap();
    assert_eq!(message.plain_text.trim(), "Plain body");
}

#[test]
fn parses_body_from_non_plain_leaf_part_when_plain_text_is_absent() {
    let raw = b"From: sender@example.com\r\nSubject: HTML only\r\nContent-Type: multipart/alternative; boundary=abc\r\n\r\n--abc\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML body</p>\r\n--abc--";
    let message = parse_inbound_message("support", 1, 3, raw).unwrap();

    assert_eq!(message.plain_text.trim(), "<p>HTML body</p>");
}

#[test]
fn forward_body_includes_metadata_and_text() {
    let message = InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 1,
            uid: 2,
            message_id: Some("<m1@example.com>".to_string()),
            in_reply_to: None,
            references: vec![],
            from_addr: "person@example.com".to_string(),
            recipients: vec![],
            subject: "Question".to_string(),
        },
        plain_text: "Original text".to_string(),
    };
    let body = forward_body("Intro", &message);
    assert_eq!(
        body,
        "Intro\n\n---------- Forwarded message ---------\nFrom: person@example.com\nSubject: Question\nMessage-ID: <m1@example.com>\nUID: 1:2\n\nOriginal text"
    );
}

#[test]
fn send_blocking_rejects_invalid_action_before_smtp() {
    let smtp = mailbox_config().smtp;
    let action = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec![],
        subject: "".to_string(),
        body: "".to_string(),
        reason: "invalid".to_string(),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    let error = crate::mail_external::send_blocking(&smtp, &action).unwrap_err();
    assert!(error.to_string().contains("recipients"));
    assert!(error.to_string().contains("subject"));
    assert!(error.to_string().contains("body"));
}

#[test]
fn parse_mailbox_reports_invalid_addresses() {
    assert!(crate::mail_external::parse_mailbox("support@example.com").is_ok());
    assert!(crate::mail_external::parse_mailbox("not an address").is_err());
}

fn filter_message(subject: &str, recipients: Vec<String>) -> InboundMessage {
    InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 1,
            uid: 2,
            message_id: Some("<m1@example.com>".to_string()),
            in_reply_to: None,
            references: vec![],
            from_addr: "person@example.com".to_string(),
            recipients,
            subject: subject.to_string(),
        },
        plain_text: "Original text".to_string(),
    }
}

fn mailbox_config() -> MailboxConfig {
    MailboxConfig {
        id: "support".to_string(),
        address: "support@example.com".to_string(),
        enabled: true,
        poll_interval_seconds: 30,
        safety_forward_to: vec!["safety@example.com".to_string()],
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
