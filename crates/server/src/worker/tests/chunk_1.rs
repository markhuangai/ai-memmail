use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::ai::{AgentDecision, AiError};
use crate::classification::{
    EmailCategory, EmailClassification, EmailRule, EmailRuleAction, EmailTaxonomy, EmailTopic,
    ResolvedEmailClassification,
};
use crate::config::{
    AcceptedCondition, AgentConfig, AiConfig, AiProtocol, BannedSenderConfig, BannedSenderKind,
    DatabaseConfig, EmailSignatureConfig, EmailSignatureFormat, ImapConfig, LoggingConfig,
    PromptConfig, ReviewConfig, SmtpConfig,
};
use crate::mail::{
    DedupeKey, MailError, MessageDirection, MessageMetadata, SentFetchBatch, SentSyncCursor,
    ThreadMessage,
};
use crate::safety::{SafetyCategory, SafetyScanResult};

use super::*;

fn config() -> AppConfig {
    AppConfig {
        version: 1,
        database: DatabaseConfig {
            host: "postgres".to_string(),
            port: 5432,
            username: "user".to_string(),
            password: "db-secret".to_string(),
            database: "ai_memmail".to_string(),
        },
        logging: LoggingConfig {
            level: "info".to_string(),
            format: "json".to_string(),
            verbose_actions: true,
            retention_days: 180,
        },
        prompts: PromptConfig {
            root: "prompts".into(),
            safety_scan: "safety.md".into(),
            email_classifier: "classifier.md".into(),
            rule_action: "rule-action.md".into(),
        },
        ai: AiConfig {
            protocol: AiProtocol::Openai,
            api_url: "https://api.example/v1".to_string(),
            api_secret: "secret".to_string(),
            model: "model".to_string(),
            review: ReviewConfig {
                enabled: false,
                prompt_path: "review.md".into(),
            },
        },
        mcp_servers: BTreeMap::new(),
        mailboxes: vec![MailboxConfig {
            id: "support".to_string(),
            address: "support@example.com".to_string(),
            enabled: true,
            poll_interval_seconds: 30,
            safety_forward_to: vec!["human@example.com".to_string()],
            signature: None,
            accepted_conditions: vec![],
            mcp_servers: vec![],
            agent: AgentConfig {
                system_prompt_path: "agent.md".into(),
                default_forward_to: vec![],
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
        }],
        banned_senders: vec![BannedSenderConfig {
            kind: BannedSenderKind::Domain,
            value: "blocked.test".to_string(),
            reason: "jailbreak attempts".to_string(),
        }],
    }
}

fn inbound(uid: u64, from_addr: &str, subject: &str, plain_text: &str) -> InboundMessage {
    InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 1,
            uid,
            message_id: Some(format!("<{uid}@example.com>")),
            in_reply_to: None,
            references: vec![],
            from_addr: from_addr.to_string(),
            recipients: vec![],
            subject: subject.to_string(),
        },
        plain_text: plain_text.to_string(),
    }
}

struct FakeMail {
    messages: Mutex<Vec<InboundMessage>>,
    sent: Mutex<Vec<OutboundAction>>,
    seen: Mutex<Vec<DedupeKey>>,
    sent_batch: Mutex<Option<SentFetchBatch>>,
    sent_fetches: Mutex<Vec<(Option<SentSyncCursor>, i64, usize)>>,
    fail_fetch: bool,
    fail_send: bool,
    fail_mark_seen: bool,
}

impl FakeMail {
    fn new(messages: Vec<InboundMessage>) -> Self {
        Self {
            messages: Mutex::new(messages),
            sent: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            sent_batch: Mutex::new(None),
            sent_fetches: Mutex::new(Vec::new()),
            fail_fetch: false,
            fail_send: false,
            fail_mark_seen: false,
        }
    }

    fn with_fail_fetch(mut self) -> Self {
        self.fail_fetch = true;
        self
    }

    fn with_fail_send(mut self) -> Self {
        self.fail_send = true;
        self
    }

    fn with_fail_mark_seen(mut self) -> Self {
        self.fail_mark_seen = true;
        self
    }

    fn with_sent_batch(self, batch: SentFetchBatch) -> Self {
        *self.sent_batch.lock().expect("sent batch lock") = Some(batch);
        self
    }

    fn sent(&self) -> Vec<OutboundAction> {
        self.sent.lock().expect("sent lock").clone()
    }

    fn seen(&self) -> Vec<DedupeKey> {
        self.seen.lock().expect("seen lock").clone()
    }

    fn sent_fetches(&self) -> Vec<(Option<SentSyncCursor>, i64, usize)> {
        self.sent_fetches.lock().expect("sent fetches lock").clone()
    }
}

#[async_trait::async_trait]
impl MailTransport for FakeMail {
    async fn fetch_unseen(
        &self,
        _mailbox: &MailboxConfig,
        _limit: usize,
    ) -> Result<Vec<InboundMessage>, MailError> {
        if self.fail_fetch {
            return Err(MailError::Imap("fetch failed".to_string()));
        }
        Ok(std::mem::take(
            &mut *self.messages.lock().expect("messages lock"),
        ))
    }

    async fn send(&self, _smtp: &SmtpConfig, action: &OutboundAction) -> Result<(), MailError> {
        if self.fail_send {
            return Err(MailError::Smtp("send failed".to_string()));
        }
        self.sent.lock().expect("sent lock").push(action.clone());
        Ok(())
    }

    async fn mark_seen(&self, mailbox: &MailboxConfig, uid: u64) -> Result<(), MailError> {
        if self.fail_mark_seen {
            return Err(MailError::Imap("mark seen failed".to_string()));
        }
        self.seen.lock().expect("seen lock").push(DedupeKey {
            mailbox_id: mailbox.id.clone(),
            uid_validity: 1,
            uid,
        });
        Ok(())
    }

    async fn fetch_sent(
        &self,
        _mailbox: &MailboxConfig,
        cursor: Option<&SentSyncCursor>,
        backfill_cutoff: i64,
        limit: usize,
    ) -> Result<SentFetchBatch, MailError> {
        self.sent_fetches.lock().expect("sent fetches lock").push((
            cursor.cloned(),
            backfill_cutoff,
            limit,
        ));
        self.sent_batch
            .lock()
            .expect("sent batch lock")
            .clone()
            .ok_or_else(|| MailError::Imap("sent fetch failed".to_string()))
    }
}

struct FakeDecisionEngine {
    scan: SafetyScanResult,
    classification: EmailClassification,
    decision: AgentDecision,
    rule_decision: AgentDecision,
    review: OutboundReviewDecision,
    fail_safety: bool,
    fail_classification: bool,
    fail_agent: bool,
    fail_rule: bool,
    fail_review: bool,
    hang_agent: bool,
    missing_classifier_prompt: bool,
    missing_rule_prompt: bool,
    context_limit_at: Option<&'static str>,
    calls: Arc<Mutex<DecisionCallCounts>>,
    reviewed_actions: Arc<Mutex<Vec<OutboundAction>>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct DecisionCallCounts {
    safety_scan: usize,
    classify_email: usize,
    agent_decision: usize,
    rule_decision: usize,
    outbound_review: usize,
}

impl FakeDecisionEngine {
    fn call_counts(&self) -> DecisionCallCounts {
        self.calls.lock().expect("decision calls lock").clone()
    }

    fn reviewed_actions(&self) -> Vec<OutboundAction> {
        self.reviewed_actions
            .lock()
            .expect("reviewed actions lock")
            .clone()
    }

    fn with_hang_agent(mut self) -> Self {
        self.hang_agent = true;
        self
    }


    fn with_context_limit_at(mut self, action: &'static str) -> Self {
        self.context_limit_at = Some(action);
        self
    }
}

#[async_trait::async_trait]
impl DecisionEngine for FakeDecisionEngine {
    fn classifier_prompt_missing(&self, _config: &AppConfig) -> Result<bool, AiError> {
        Ok(self.missing_classifier_prompt)
    }

    fn rule_prompt_missing(&self, _config: &AppConfig) -> Result<bool, AiError> {
        Ok(self.missing_rule_prompt)
    }

    async fn safety_scan(
        &self,
        _config: &AppConfig,
        _mailbox: &MailboxConfig,
        _message: &InboundMessage,
    ) -> Result<SafetyScanResult, AiError> {
        self.calls.lock().expect("decision calls lock").safety_scan += 1;
        if self.context_limit_at == Some("safety_scan") {
            return Err(AiError::ContextLengthExceeded("provider limit".to_string()));
        }
        if self.fail_safety {
            return Err(AiError::Provider("safety failed".to_string()));
        }
        Ok(self.scan.clone())
    }

    async fn classify_email(
        &self,
        _config: &AppConfig,
        _mailbox: &MailboxConfig,
        _message: &InboundMessage,
        _thread_context: &ThreadContext,
        _taxonomy: &EmailTaxonomy,
    ) -> Result<EmailClassification, AiError> {
        self.calls
            .lock()
            .expect("decision calls lock")
            .classify_email += 1;
        if self.context_limit_at == Some("email_classification") {
            return Err(AiError::ContextLengthExceeded("provider limit".to_string()));
        }
        if self.fail_classification {
            return Err(AiError::Provider("classification failed".to_string()));
        }
        Ok(self.classification.clone())
    }

    async fn agent_decision(
        &self,
        _config: &AppConfig,
        _mailbox: &MailboxConfig,
        _message: &InboundMessage,
        _thread_context: &ThreadContext,
    ) -> Result<AgentDecision, AiError> {
        self.calls
            .lock()
            .expect("decision calls lock")
            .agent_decision += 1;
        if self.context_limit_at == Some("agent_decision") {
            return Err(AiError::ContextLengthExceeded("provider limit".to_string()));
        }
        if self.fail_agent {
            return Err(AiError::Provider("agent failed".to_string()));
        }
        if self.hang_agent {
            tokio::time::sleep(WORKER_STEP_TIMEOUT + Duration::from_millis(25)).await;
        }
        Ok(self.decision.clone())
    }

    async fn rule_decision(
        &self,
        _config: &AppConfig,
        _mailbox: &MailboxConfig,
        _message: &InboundMessage,
        _thread_context: &ThreadContext,
        _classification: &EmailClassification,
        _rule: &EmailRule,
    ) -> Result<AgentDecision, AiError> {
        self.calls
            .lock()
            .expect("decision calls lock")
            .rule_decision += 1;
        if self.context_limit_at == Some("rule_decision") {
            return Err(AiError::ContextLengthExceeded("provider limit".to_string()));
        }
        if self.fail_rule {
            return Err(AiError::Provider("rule failed".to_string()));
        }
        Ok(self.rule_decision.clone())
    }

    async fn outbound_review(
        &self,
        _config: &AppConfig,
        _mailbox: &MailboxConfig,
        _message: &InboundMessage,
        _thread_context: &ThreadContext,
        decision: &AgentDecision,
    ) -> Result<OutboundReviewDecision, AiError> {
        self.calls
            .lock()
            .expect("decision calls lock")
            .outbound_review += 1;
        if self.context_limit_at == Some("outbound_review") {
            return Err(AiError::ContextLengthExceeded("provider limit".to_string()));
        }
        self.reviewed_actions
            .lock()
            .expect("reviewed actions lock")
            .push(decision.action.clone());
        if self.fail_review {
            return Err(AiError::Provider("review failed".to_string()));
        }
        Ok(self.review.clone())
    }
}

enum FakeClaimOutcome {
    Claimed,
    InProgress,
    AlreadyFinished,
    Fail,
}

struct FakeProcessingStore {
    claims: Mutex<Vec<FakeClaimOutcome>>,
    run_ids: Mutex<Vec<String>>,
    fail_update: bool,
    fail_taxonomy: bool,
    fail_resolve: bool,
    fail_rule_match: bool,
    fail_classification_record: bool,
    touches: Mutex<usize>,
    statuses: Mutex<Vec<String>>,
    matched_rule: Option<EmailRule>,
    thread_context: Option<ThreadContext>,
    handoff: Option<crate::storage::ThreadHandoff>,
    handoff_deliveries: Mutex<Vec<NewThreadHandoffDelivery>>,
}

impl FakeProcessingStore {
    fn new(claims: Vec<FakeClaimOutcome>) -> Self {
        Self {
            claims: Mutex::new(claims),
            run_ids: Mutex::new(Vec::new()),
            fail_update: false,
            fail_taxonomy: false,
            fail_resolve: false,
            fail_rule_match: false,
            fail_classification_record: false,
            touches: Mutex::new(0),
            statuses: Mutex::new(Vec::new()),
            matched_rule: None,
            thread_context: None,
            handoff: None,
            handoff_deliveries: Mutex::new(Vec::new()),
        }
    }

    fn with_fail_update(mut self) -> Self {
        self.fail_update = true;
        self
    }

    fn with_fail_taxonomy(mut self) -> Self {
        self.fail_taxonomy = true;
        self
    }

    fn with_fail_resolve(mut self) -> Self {
        self.fail_resolve = true;
        self
    }

    fn with_fail_rule_match(mut self) -> Self {
        self.fail_rule_match = true;
        self
    }

    fn with_fail_classification_record(mut self) -> Self {
        self.fail_classification_record = true;
        self
    }

    fn with_matched_rule(mut self, rule: EmailRule) -> Self {
        self.matched_rule = Some(rule);
        self
    }

    fn with_thread_context(mut self, context: ThreadContext) -> Self {
        self.thread_context = Some(context);
        self
    }

    fn with_handoff(mut self, handoff: crate::storage::ThreadHandoff) -> Self {
        self.handoff = Some(handoff);
        self
    }

    fn run_ids(&self) -> Vec<String> {
        self.run_ids.lock().expect("run ids lock").clone()
    }

    fn touch_count(&self) -> usize {
        *self.touches.lock().expect("touches lock")
    }

    fn statuses(&self) -> Vec<String> {
        self.statuses.lock().expect("statuses lock").clone()
    }

    fn handoff_deliveries(&self) -> Vec<NewThreadHandoffDelivery> {
        self.handoff_deliveries
            .lock()
            .expect("handoff deliveries lock")
            .clone()
    }
}

#[async_trait::async_trait]
impl ProcessingStore for FakeProcessingStore {
    async fn claim_message(
        &self,
        run_id: &str,
        _message: &InboundMessage,
    ) -> Result<ProcessingClaim, crate::storage::StorageError> {
        self.run_ids
            .lock()
            .map_err(|_| crate::storage::StorageError::LockPoisoned)?
            .push(run_id.to_string());
        let outcome = self
            .claims
            .lock()
            .map_err(|_| crate::storage::StorageError::LockPoisoned)?
            .remove(0);
        match outcome {
            FakeClaimOutcome::Claimed => Ok(ProcessingClaim::Claimed),
            FakeClaimOutcome::InProgress => Ok(ProcessingClaim::InProgress {
                status: "processing".to_string(),
            }),
            FakeClaimOutcome::AlreadyFinished => Ok(ProcessingClaim::AlreadyFinished {
                status: "replied".to_string(),
            }),
            FakeClaimOutcome::Fail => Err(crate::storage::StorageError::LockPoisoned),
        }
    }

    async fn update_message_status(
        &self,
        _key: &DedupeKey,
        status: &str,
        _outbound_action: Option<&OutboundActionKind>,
    ) -> Result<(), crate::storage::StorageError> {
        if self.fail_update {
            return Err(crate::storage::StorageError::LockPoisoned);
        }
        self.statuses
            .lock()
            .map_err(|_| crate::storage::StorageError::LockPoisoned)?
            .push(status.to_string());
        Ok(())
    }

    async fn touch_processing(
        &self,
        _key: &DedupeKey,
    ) -> Result<(), crate::storage::StorageError> {
        *self
            .touches
            .lock()
            .map_err(|_| crate::storage::StorageError::LockPoisoned)? += 1;
        Ok(())
    }

    async fn record_safety_result(
        &self,
        _key: &DedupeKey,
        _category: &SafetyCategory,
        _reason: &str,
    ) -> Result<(), crate::storage::StorageError> {
        Ok(())
    }

    async fn upsert_sender_review(
        &self,
        _sender: &str,
        _mailbox_id: &str,
        _reason: &str,
    ) -> Result<(), crate::storage::StorageError> {
        Ok(())
    }

    async fn active_email_taxonomy(
        &self,
    ) -> Result<EmailTaxonomy, crate::storage::StorageError> {
        if self.fail_taxonomy {
            return Err(crate::storage::StorageError::LockPoisoned);
        }
        Ok(test_taxonomy())
    }

    async fn resolve_email_classification(
        &self,
        classification: &EmailClassification,
    ) -> Result<ResolvedEmailClassification, crate::storage::StorageError> {
        if self.fail_resolve {
            return Err(crate::storage::StorageError::InvalidClassification(
                "cannot resolve model category".to_string(),
            ));
        }
        Ok(resolved_classification(classification))
    }

    async fn find_matching_email_rule(
        &self,
        _mailbox_id: &str,
        _classification: &ResolvedEmailClassification,
    ) -> Result<Option<EmailRule>, crate::storage::StorageError> {
        if self.fail_rule_match {
            return Err(crate::storage::StorageError::LockPoisoned);
        }
        Ok(self.matched_rule.clone())
    }

    async fn record_email_classification(
        &self,
        _key: &DedupeKey,
        _classification: &ResolvedEmailClassification,
        _decision_source: &str,
        _matched_rule: Option<&EmailRule>,
    ) -> Result<(), crate::storage::StorageError> {
        if self.fail_classification_record {
            return Err(crate::storage::StorageError::LockPoisoned);
        }
        Ok(())
    }

    async fn load_thread_context(
        &self,
        _mailbox: &MailboxConfig,
        message: &InboundMessage,
    ) -> Result<ThreadContext, crate::storage::StorageError> {
        Ok(self
            .thread_context
            .clone()
            .unwrap_or_else(|| ThreadContext::empty(message.metadata.thread_id())))
    }

    async fn active_thread_handoff(
        &self,
        mailbox_id: &str,
        thread_id: &str,
    ) -> Result<Option<crate::storage::ThreadHandoff>, crate::storage::StorageError> {
        Ok(self
            .handoff
            .clone()
            .filter(|handoff| handoff.mailbox_id == mailbox_id && handoff.thread_id == thread_id))
    }

    async fn begin_thread_handoff_delivery(
        &self,
        delivery: &NewThreadHandoffDelivery,
    ) -> Result<crate::storage::ThreadHandoffDelivery, crate::storage::StorageError> {
        self.handoff_deliveries
            .lock()
            .map_err(|_| crate::storage::StorageError::LockPoisoned)?
            .push(delivery.clone());
        Ok(crate::storage::ThreadHandoffDelivery {
            request_id: delivery.request_id,
            mailbox_id: delivery.mailbox_id.clone(),
            thread_id: delivery.thread_id.clone(),
            source_run_id: delivery.source_run_id,
            destination: delivery.destination.clone(),
            remote_target: delivery.remote_target.clone(),
            outbound_message_id: delivery.outbound_message_id.clone(),
            status: "sending".to_string(),
            error: None,
        })
    }

    async fn finish_thread_handoff_delivery(
        &self,
        _mailbox_id: &str,
        _thread_id: &str,
        _request_id: uuid::Uuid,
        _status: &str,
        _error: Option<&str>,
    ) -> Result<(), crate::storage::StorageError> {
        Ok(())
    }
}
