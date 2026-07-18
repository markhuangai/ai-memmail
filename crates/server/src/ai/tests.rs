use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::config::{
    AgentConfig, AiConfig, AiProtocol, DatabaseConfig, ImapConfig, LoggingConfig, MailboxConfig,
    McpServerConfig, McpTransport, PromptConfig, ReviewConfig, SmtpConfig,
};
use crate::mail::MessageMetadata;

#[derive(Clone)]
struct FakeChatProvider {
    responses: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<Vec<Value>>>>,
}

impl FakeChatProvider {
    fn new(responses: Vec<&str>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(
                responses
                    .into_iter()
                    .map(str::to_string)
                    .collect::<VecDeque<_>>(),
            )),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<Vec<Value>> {
        self.requests.lock().expect("chat requests").clone()
    }
}

#[async_trait]
impl ChatProvider for FakeChatProvider {
    async fn chat(&self, _config: &AppConfig, messages: Vec<Value>) -> Result<String, AiError> {
        self.requests.lock().expect("chat requests").push(messages);
        self.responses
            .lock()
            .expect("chat responses")
            .pop_front()
            .ok_or_else(|| AiError::Provider("no fake response queued".to_string()))
    }
}

#[derive(Clone)]
struct FakeMcpContextProvider {
    context: String,
    calls: Arc<Mutex<usize>>,
}

impl FakeMcpContextProvider {
    fn new(context: &str) -> Self {
        Self {
            context: context.to_string(),
            calls: Arc::new(Mutex::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        *self.calls.lock().expect("mcp calls")
    }
}

#[async_trait]
impl McpContextProvider for FakeMcpContextProvider {
    async fn recall_mailbox_context(
        &self,
        _config: &AppConfig,
        _mailbox: &MailboxConfig,
        _message: &InboundMessage,
    ) -> Result<String, AiError> {
        *self.calls.lock().expect("mcp calls") += 1;
        Ok(self.context.clone())
    }
}

#[derive(Clone, Default)]
struct FakeBlockingChatClient {
    requests: Arc<Mutex<Vec<Vec<Value>>>>,
}

impl FakeBlockingChatClient {
    fn requests(&self) -> Vec<Vec<Value>> {
        self.requests
            .lock()
            .expect("blocking chat requests")
            .clone()
    }
}

impl BlockingChatClient for FakeBlockingChatClient {
    fn chat(&self, _config: &AppConfig, messages: Vec<Value>) -> Result<String, AiError> {
        self.requests
            .lock()
            .expect("blocking chat requests")
            .push(messages);
        Ok("blocking chat response".to_string())
    }
}

#[derive(Clone)]
struct FakeBlockingMcpClient {
    fail_assemble: bool,
    calls: Arc<Mutex<Vec<String>>>,
    queries: Arc<Mutex<Vec<String>>>,
}

impl FakeBlockingMcpClient {
    fn new(fail_assemble: bool) -> Self {
        Self {
            fail_assemble,
            calls: Arc::new(Mutex::new(Vec::new())),
            queries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("blocking mcp calls").clone()
    }

    fn queries(&self) -> Vec<String> {
        self.queries.lock().expect("blocking mcp queries").clone()
    }
}

impl BlockingMcpClient for FakeBlockingMcpClient {
    fn call(
        &self,
        _url: &str,
        _api_key: &str,
        _method: &str,
        params: Value,
    ) -> Result<Value, AiError> {
        let name = params["name"].as_str().unwrap_or_default().to_string();
        self.calls
            .lock()
            .expect("blocking mcp calls")
            .push(name.clone());
        self.queries.lock().expect("blocking mcp queries").push(
            params["arguments"]["query"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
        if self.fail_assemble && name == "assemble_context" {
            return Err(AiError::Mcp("assemble failed".to_string()));
        }
        Ok(serde_json::json!({
            "result": {
                "content": [
                    {"type": "text", "text": "fallback memory says 90% coverage"}
                ]
            }
        }))
    }
}

fn inbound(subject: &str, plain_text: &str) -> InboundMessage {
    InboundMessage {
        metadata: MessageMetadata {
            mailbox_id: "support".to_string(),
            uid_validity: 1,
            uid: 2,
            message_id: None,
            in_reply_to: None,
            references: vec![],
            from_addr: "person@example.com".to_string(),
            recipients: vec![],
            subject: subject.to_string(),
        },
        plain_text: plain_text.to_string(),
    }
}

fn app_config() -> AppConfig {
    AppConfig {
        version: 1,
        database: DatabaseConfig {
            host: "postgres".to_string(),
            port: 5432,
            username: "user".to_string(),
            password: "secret".to_string(),
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
            email_classifier: "email-classifier.md".into(),
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
        mailboxes: vec![mailbox_config()],
        banned_senders: vec![],
    }
}

fn mailbox_config() -> MailboxConfig {
    MailboxConfig {
        id: "support".to_string(),
        address: "support@example.com".to_string(),
        enabled: true,
        poll_interval_seconds: 30,
        safety_forward_to: vec!["safety@example.com".to_string()],
        signature: None,
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

fn email_rule(action: crate::classification::EmailRuleAction) -> EmailRule {
    EmailRule {
        id: 1,
        mailbox_id: "support".to_string(),
        name: "Auto-decline marketing/vendor outreach".to_string(),
        category_id: 1,
        category: "marketing_vendor".to_string(),
        topic_ids: vec![],
        topics: vec![],
        action,
        reply_goal: "Politely decline paid marketing services.".to_string(),
        enabled: true,
        priority: 100,
        created_at: "test".to_string(),
        updated_at: "test".to_string(),
    }
}

fn write_prompt_root(name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let root =
        std::env::temp_dir().join(format!("ai-memmail-{name}-{}-{millis}", std::process::id()));
    fs::create_dir_all(&root).expect("create prompt root");
    fs::write(root.join("safety.md"), "Safety system prompt").expect("write safety prompt");
    fs::write(root.join("agent.md"), "Agent system prompt").expect("write agent prompt");
    fs::write(root.join("review.md"), "Review system prompt").expect("write review prompt");
    fs::write(root.join("email-classifier.md"), "Classifier system prompt")
        .expect("write classifier prompt");
    fs::write(root.join("rule-action.md"), "Rule action system prompt")
        .expect("write rule action prompt");
    root
}

mod chunk_1;
mod chunk_2;
