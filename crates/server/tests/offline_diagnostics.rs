use std::process::Command;

#[test]
fn startup_parse_failure_reports_safe_location_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.yaml");
    std::fs::write(&path, "version: synthetic-private-credential\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ai-memmail-server"))
        .env("AI_MEMMAIL_CONFIG", &path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("level=fatal action=startup status=failed"));
    assert!(error.contains("version"));
    assert!(error.contains("invalid field type"));
    assert!(error.contains("line"));
    assert!(!error.contains("synthetic-private-credential"));
}

#[test]
fn startup_regex_validation_reports_field_and_cause_without_pattern() {
    let mut config = ai_memmail_server::config::AppConfig::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/config.example.yaml"),
    )
    .unwrap();
    config.mailboxes[0]
        .accepted_conditions
        .push(ai_memmail_server::config::AcceptedCondition {
            recipients: vec![],
            subject_regex: vec!["(synthetic-private-credential".to_string()],
        });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid-regex.yaml");
    std::fs::write(&path, yaml_serde::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ai-memmail-server"))
        .env("AI_MEMMAIL_CONFIG", &path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("level=fatal action=startup status=failed"));
    assert!(error.contains("accepted_conditions"));
    assert!(error.contains("subject_regex"));
    assert!(error.contains("regex syntax error"));
    assert!(error.contains("pattern omitted"));
    assert!(!error.contains("synthetic-private-credential"));
}
