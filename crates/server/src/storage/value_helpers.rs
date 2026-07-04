pub fn processing_status_can_reclaim(status: &str, stale: bool) -> bool {
    status == PROCESSING_STATUS_SEND_FAILED
        || status == PROCESSING_STATUS_RETRYABLE_FAILED
        || (status == PROCESSING_STATUS_PROCESSING && stale)
}

pub fn processing_claim_for_existing_status(status: &str) -> ProcessingClaim {
    if status == PROCESSING_STATUS_PROCESSING {
        ProcessingClaim::InProgress {
            status: status.to_string(),
        }
    } else {
        ProcessingClaim::AlreadyFinished {
            status: status.to_string(),
        }
    }
}

pub fn outbound_action_value(kind: &OutboundActionKind) -> &'static str {
    match kind {
        OutboundActionKind::Reply => "reply",
        OutboundActionKind::Forward => "forward",
        OutboundActionKind::Noop => "noop",
    }
}

pub fn safety_category_value(category: &SafetyCategory) -> &'static str {
    match category {
        SafetyCategory::Safe => "safe",
        SafetyCategory::Jailbreak => "jailbreak",
        SafetyCategory::PromptInjection => "prompt_injection",
        SafetyCategory::Hacking => "hacking",
        SafetyCategory::SensitiveExfiltration => "sensitive_exfiltration",
        SafetyCategory::Unknown => "unknown",
    }
}

pub(crate) fn outbound_body_for_storage(action: &OutboundAction) -> (Option<&str>, bool) {
    match action.kind {
        OutboundActionKind::Reply => (Some(action.body.as_str()), false),
        OutboundActionKind::Forward => (None, true),
        OutboundActionKind::Noop => (None, false),
    }
}

pub(crate) fn inbound_body_for_storage(message: &InboundMessage) -> (String, bool) {
    let mut output = String::new();
    let mut truncated = false;
    for (index, character) in message.plain_text.chars().enumerate() {
        if index >= INBOUND_BODY_STORAGE_MAX_CHARS {
            truncated = true;
            break;
        }
        output.push(character);
    }
    (output, truncated)
}

pub(crate) fn empty_string_as_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn metadata_only_schema_guard(sql: &str) -> Result<(), String> {
    let lowered = sql.to_ascii_lowercase();
    let forbidden = [
        " raw_email",
        "email_body",
        " body text",
        " body bytea",
        " parsed_content",
        " message_content",
    ];
    for token in forbidden {
        if lowered.contains(token) {
            return Err(format!(
                "migration contains forbidden email-content column token: {token}"
            ));
        }
    }
    Ok(())
}

pub fn retention_delete_sql(retention_days: u16) -> String {
    format!(
        "DELETE FROM action_logs WHERE created_at < now() - interval '{} days'",
        retention_days
    )
}

pub(crate) fn log_level_value(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
        LogLevel::Fatal => "fatal",
    }
}
