use super::*;

#[test]
fn yaml_codec_preserves_literal_strings_signatures_and_optional_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    for literal in ["null", "true", "09", "# not a comment", "first\nsecond\n"] {
        let mut config = valid_config();
        config.ai.api_secret = literal.to_string();
        config.mailboxes[0].imap.sent_folder = Some(literal.to_string());
        config.mailboxes[0].signature = Some(EmailSignatureConfig {
            format: EmailSignatureFormat::Html,
            content: "<table><tr><td style=\"color: red\">Mark<br>Support</td></tr></table>"
                .to_string(),
        });
        config.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap(), config.sanitized_for_save());
        let loaded = AppConfig::load(&path).unwrap();
        loaded.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap(), loaded);
    }
}

#[test]
fn yaml_errors_explain_field_and_location_without_input_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.yaml");
    let config = valid_config();
    let yaml = yaml_serde::to_string(&config)
        .unwrap()
        .replace("protocol: openai", "protocol: synthetic-private-credential");
    std::fs::write(&path, yaml).unwrap();
    let error = AppConfig::load(&path).unwrap_err().to_string();
    assert!(error.contains("ai.protocol"), "{error}");
    assert!(error.contains("unsupported field option"), "{error}");
    assert!(error.contains("line"));
    assert!(error.contains("column"));
    assert!(!error.contains("synthetic-private-credential"));
    assert!(!error.contains("db-secret"));
    assert!(!error.contains("imap-secret"));
}
