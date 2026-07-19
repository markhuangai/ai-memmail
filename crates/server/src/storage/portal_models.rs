#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PortalConversationSummary {
    pub conversation_id: uuid::Uuid,
    pub mailbox_id: String,
    pub thread_id: String,
    pub subject: String,
    pub revision: i64,
    pub last_message_at: String,
    pub latest_sender: String,
    pub latest_status: String,
    pub remote_reply_to: Option<String>,
    pub unsafe_reply_requires_confirmation: bool,
    pub source_conversation_id: Option<uuid::Uuid>,
    pub handoff: Option<ThreadHandoffSummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PortalConversationDetail {
    pub conversation: PortalConversationSummary,
    pub messages: Vec<PortalTimelineMessage>,
    pub quote_text: String,
    pub quote_html: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PortalTimelineMessage {
    pub id: String,
    pub direction: String,
    pub kind: String,
    pub status: String,
    pub from_addr: String,
    pub to_recipients: Vec<String>,
    pub cc_recipients: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bcc_recipients: Vec<String>,
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub body_truncated: bool,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub safety_category: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPortalMessage {
    pub portal_message_id: uuid::Uuid,
    pub conversation_id: uuid::Uuid,
    pub request_id: uuid::Uuid,
    pub mailbox_id: String,
    pub thread_id: String,
    pub action: String,
    pub to_recipients: Vec<String>,
    pub cc_recipients: Vec<String>,
    pub bcc_recipients: Vec<String>,
    pub subject: String,
    pub authored_text: String,
    pub authored_html: Option<String>,
    pub rendered_text: String,
    pub rendered_html: Option<String>,
    pub quoted_text: String,
    pub quoted_html: Option<String>,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub reply_target: Option<String>,
    pub source_conversation_id: Option<uuid::Uuid>,
    pub child_conversation_id: Option<uuid::Uuid>,
    pub unsafe_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PortalMessageRecord {
    pub portal_message_id: uuid::Uuid,
    pub conversation_id: uuid::Uuid,
    pub request_id: uuid::Uuid,
    pub action: String,
    pub status: String,
    pub to_recipients: Vec<String>,
    pub cc_recipients: Vec<String>,
    pub bcc_recipients: Vec<String>,
    pub subject: String,
    pub authored_text: String,
    pub authored_html: Option<String>,
    pub rendered_text: String,
    pub rendered_html: Option<String>,
    pub quoted_text: String,
    pub quoted_html: Option<String>,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub reply_target: Option<String>,
    pub child_conversation_id: Option<uuid::Uuid>,
    pub error: Option<String>,
}
