use crate::config::{EmailSignatureConfig, EmailSignatureFormat};

use super::*;

#[test]
fn validates_html_body_only_for_nonempty_replies() {
    let mut reply = OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Hello".to_string(),
        body: "Thanks".to_string(),
        html_body: Some("   ".to_string()),
        reason: "known answer".to_string(),
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };
    let errors = validate_outbound_action(&reply).unwrap_err();
    assert_eq!(errors[0].field, "html_body");

    reply.html_body = Some("<p>Thanks</p>".to_string());
    assert!(validate_outbound_action(&reply).is_ok());

    reply.kind = OutboundActionKind::Noop;
    reply.recipients.clear();
    reply.subject.clear();
    reply.body.clear();
    let errors = validate_outbound_action(&reply).unwrap_err();
    assert_eq!(errors[0].field, "html_body");
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
fn apply_reply_signature_preserves_legacy_notice_when_no_signature_is_configured() {
    let mailbox = mailbox_config();
    let mut action = reply_action("Answer");

    apply_reply_signature(&mailbox, &mut action);

    assert_eq!(
        action.body,
        "Answer\n\n--\nThis automated reply was sent on Mark's behalf. If this needs Mark's attention, reply with: escalation to human"
    );
    assert_eq!(action.html_body, None);
}

#[test]
fn apply_reply_signature_appends_plain_text_signature_without_notice() {
    let mut mailbox = mailbox_config();
    mailbox.signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::PlainText,
        content: "--\nMark".to_string(),
    });
    let mut action = reply_action("Answer\n");

    apply_reply_signature(&mailbox, &mut action);

    assert_eq!(action.body, "Answer\n\n--\nMark");
    assert_eq!(action.html_body, None);
    assert!(!action.body.contains(AUTOMATED_REPLY_NOTICE));
}

#[test]
fn apply_reply_signature_escapes_ai_reply_and_archives_final_html() {
    let mut mailbox = mailbox_config();
    mailbox.signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><strong>Mark</strong><img src="https://example.com/sig.png"></p>"#
            .to_string(),
    });
    let mut action = reply_action("A&B\r\n<ok> \"yes\" 'now'\n");

    apply_reply_signature(&mailbox, &mut action);

    assert_eq!(action.body, "A&B\r\n<ok> \"yes\" 'now'\n");
    assert_eq!(
        action.html_body.as_deref(),
        Some(
            "A&amp;B<br>&lt;ok&gt; &quot;yes&quot; &#39;now&#39;<br><br><p><strong>Mark</strong><img src=\"https://example.com/sig.png\"></p>"
        )
    );
}

#[test]
fn apply_reply_signature_clears_html_body_on_non_replies() {
    let mailbox = mailbox_config();
    let mut action = OutboundAction {
        kind: OutboundActionKind::Forward,
        recipients: vec!["human@example.com".to_string()],
        subject: "Fwd: Question".to_string(),
        body: "Please review".to_string(),
        html_body: Some("<p>stale</p>".to_string()),
        reason: "human review".to_string(),
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };

    apply_reply_signature(&mailbox, &mut action);

    assert_eq!(action.body, "Please review");
    assert_eq!(action.html_body, None);
}

#[test]
fn html_signature_converts_lone_carriage_returns_to_breaks() {
    let mut mailbox = mailbox_config();
    mailbox.signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: "<p>Mark</p>".to_string(),
    });
    let mut action = reply_action("Line 1\rLine 2");

    apply_reply_signature(&mailbox, &mut action);

    assert_eq!(
        action.html_body.as_deref(),
        Some("Line 1<br>Line 2<br><br><p>Mark</p>")
    );
}

fn reply_action(body: &str) -> OutboundAction {
    OutboundAction {
        kind: OutboundActionKind::Reply,
        recipients: vec!["person@example.com".to_string()],
        subject: "Re: Question".to_string(),
        body: body.to_string(),
        html_body: None,
        reason: "known answer".to_string(),
        reply_to: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    }
}
