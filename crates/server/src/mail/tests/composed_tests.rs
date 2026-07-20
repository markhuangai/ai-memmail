use super::*;

#[test]
fn validates_composed_email_requires_recipient_subject_and_body() {
    let message = ComposedEmail {
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "".to_string(),
        text_body: "".to_string(),
        html_body: Some("".to_string()),
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };

    let errors = validate_composed_email(&message).unwrap_err();

    assert_eq!(errors.len(), 4);
    assert_eq!(errors[0].field, "recipients");
    assert_eq!(errors[3].field, "html_body");
}

#[test]
fn validates_composed_email_rejects_malformed_recipients() {
    let message = ComposedEmail {
        to: vec!["bad recipient".to_string()],
        cc: vec!["".to_string()],
        bcc: vec!["Hidden <hidden@example.com>".to_string()],
        subject: "Question".to_string(),
        text_body: "Answer".to_string(),
        html_body: None,
        message_id: None,
        in_reply_to: None,
        references: vec![],
    };

    let errors = validate_composed_email(&message).unwrap_err();

    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].field, "to[0]");
    assert_eq!(errors[0].message, "invalid email address");
    assert_eq!(errors[1].field, "cc[0]");
    assert_eq!(errors[1].message, "recipient is required");
}
