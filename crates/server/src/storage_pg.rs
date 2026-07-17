use std::collections::{HashMap, HashSet};

use crate::ai::AgentDecision;
use crate::classification::{
    default_categories, default_topics, normalize_label_name, EmailCategory, EmailClassification,
    EmailClassificationConfig, EmailRule, EmailRuleAction, EmailTaxonomy, EmailTopic, NewEmailRule,
    ResolvedEmailClassification, DEFAULT_MARKETING_REPLY_GOAL,
};
use crate::config::{AppConfig, DatabaseConfig, MailboxConfig};
use crate::logging::{ActionEvent, ActionLogger};
use crate::mail::{
    extract_authored_text, DedupeKey, InboundMessage, MessageDirection, OutboundAction,
    OutboundActionKind, SentFetchBatch, SentSyncCursor, ThreadContext, ThreadMessage,
};
use crate::safety::SafetyCategory;
use crate::storage::{
    empty_string_as_none, inbound_body_for_storage, log_level_value, migration_checksum,
    outbound_action_value, outbound_body_for_storage, parse_run_id,
    processing_claim_for_existing_status, processing_status_can_reclaim, safety_category_value,
    validate_applied_migration, AppliedMigration, Migration, NewThreadHandoffDelivery,
    ProcessedEmail, ProcessedEmailLog, ProcessingClaim, ProcessingStore, SentSyncState,
    StorageError, ThreadHandoff, ThreadHandoffDelivery, ThreadHandoffSource, ThreadHandoffSummary,
    MIGRATIONS, MIGRATION_LOCK_ID, PROCESSING_STALE_AFTER_MINUTES, PROCESSING_STATUS_PROCESSING,
    PROCESSING_STATUS_RETRYABLE_FAILED, PROCESSING_STATUS_SEND_FAILED, SCHEMA_MIGRATIONS_SQL,
};

#[derive(Debug)]
pub struct PgStore {
    client: tokio_postgres::Client,
}

include!("storage_pg/core.rs");

include!("storage_pg/sent_store_impl.rs");

include!("storage_pg/handoff.rs");

include!("storage_pg/classification.rs");

include!("storage_pg/rows.rs");

include!("storage_pg/processing_store.rs");

#[async_trait::async_trait]
impl ActionLogger for PgStore {
    async fn log(&self, event: ActionEvent) {
        if let Err(error) = self.insert_action_log(&event).await {
            tracing::error!(%error, ?event, "failed to persist action log");
        }
    }
}

#[cfg(test)]
mod tests;
