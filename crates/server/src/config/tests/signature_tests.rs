use super::*;

#[test]
fn validates_mailbox_signature_content_when_configured() {
    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: "   ".to_string(),
    });

    assert_invalid_config(config, "signature.content must not be empty");
}

#[test]
fn rejects_visually_empty_html_mailbox_signature() {
    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: "<p><br></p>".to_string(),
    });

    assert_invalid_config(config, "signature.content must not be visually empty");
}

#[test]
fn validates_html_mailbox_signature_images() {
    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><img src="http://example.com/logo.png" alt="Logo"></p>"#.to_string(),
    });

    assert_invalid_config(config, "image src must start with https://");

    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><img src="https://example.com/logo.png" alt=" "></p>"#.to_string(),
    });

    assert_invalid_config(config, "image alt text must not be empty");

    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><img src="https://example.com/logo.png" alt="Logo"></p>"#.to_string(),
    });

    assert!(config.validate().is_ok());
}

#[test]
fn accepts_visible_html_signature_text_and_plain_text_signature() {
    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: "<p>Mark&nbsp;</p>".to_string(),
    });

    assert!(config.validate().is_ok());

    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::PlainText,
        content: "Mark".to_string(),
    });

    assert!(config.validate().is_ok());
}

#[test]
fn validates_html_mailbox_signature_image_attribute_variants() {
    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><IMG src='HTTPS://example.com/logo.png' alt='Logo'></p>"#.to_string(),
    });

    assert!(config.validate().is_ok());

    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><img src=https://example.com/logo.png alt=Logo></p>"#.to_string(),
    });

    assert!(config.validate().is_ok());

    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><image src="http://example.com/logo.png"></image>Mark</p>"#.to_string(),
    });

    assert!(config.validate().is_ok());
}

#[test]
fn rejects_html_mailbox_signature_images_without_src() {
    let mut config = valid_config();
    config.mailboxes[0].signature = Some(EmailSignatureConfig {
        format: EmailSignatureFormat::Html,
        content: r#"<p><img alt="Logo"></p>"#.to_string(),
    });

    assert_invalid_config(config, "image src must start with https://");
}
