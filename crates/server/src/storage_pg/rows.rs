fn email_category_from_row(row: tokio_postgres::Row) -> EmailCategory {
    EmailCategory {
        id: row.get(0),
        name: row.get(1),
        description: row.get(2),
        status: row.get(3),
        source: row.get(4),
        created_at: row.get(5),
        updated_at: row.get(6),
    }
}

fn email_topic_from_row(row: tokio_postgres::Row) -> EmailTopic {
    EmailTopic {
        id: row.get(0),
        name: row.get(1),
        description: row.get(2),
        status: row.get(3),
        source: row.get(4),
        created_at: row.get(5),
        updated_at: row.get(6),
    }
}

fn email_rule_from_row(row: tokio_postgres::Row) -> Result<EmailRule, StorageError> {
    let action: String = row.get(7);
    Ok(EmailRule {
        id: row.get(0),
        mailbox_id: row.get(1),
        name: row.get(2),
        category_id: row.get(3),
        category: row.get(4),
        topic_ids: row.get(5),
        topics: row.get(6),
        action: EmailRuleAction::try_from(action.as_str())
            .map_err(StorageError::InvalidClassification)?,
        reply_goal: row.get(8),
        enabled: row.get(9),
        priority: row.get(10),
        created_at: row.get(11),
        updated_at: row.get(12),
    })
}

fn validate_label_status(status: &str) -> Result<(), StorageError> {
    if matches!(status, "active" | "archived") {
        Ok(())
    } else {
        Err(StorageError::InvalidClassification(format!(
            "label status must be active or archived, got {status}"
        )))
    }
}

fn validate_rule(rule: &NewEmailRule) -> Result<(), StorageError> {
    if rule.mailbox_id.trim().is_empty() {
        return Err(StorageError::InvalidClassification(
            "rule mailbox_id is required".to_string(),
        ));
    }
    if rule.name.trim().is_empty() {
        return Err(StorageError::InvalidClassification(
            "rule name is required".to_string(),
        ));
    }
    if !matches!(rule.action, EmailRuleAction::Noop) && rule.reply_goal.trim().is_empty() {
        return Err(StorageError::InvalidClassification(
            "reply_goal is required for reply and forward rules".to_string(),
        ));
    }
    Ok(())
}
