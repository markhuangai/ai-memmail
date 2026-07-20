use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{AppConfig, MailboxConfig};
use crate::html_sanitizer::sanitize_email_html;
use crate::mail::{
    outbound_message_id, reply_recipient, validate_composed_email, ComposedEmail, ValidationError,
};
use crate::storage::{
    NewPortalMessage, PgStore, PortalConversationDetail, PortalConversationSummary,
    PortalTimelineMessage,
};

use super::{message_limit, require_auth, ApiError, AppState, MessagesQuery};

#[derive(Debug, Serialize)]
pub struct ConversationsResponse {
    pub conversations: Vec<PortalConversationSummary>,
}

#[derive(Debug, Serialize)]
pub struct ConversationResponse {
    pub conversation: PortalConversationDetail,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortalSendAction {
    Reply,
    Forward,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct PortalSendRequest {
    pub request_id: Uuid,
    pub thread_revision: i64,
    pub action: PortalSendAction,
    #[serde(default)]
    pub authored_text: String,
    #[serde(default)]
    pub authored_html: Option<String>,
    #[serde(default)]
    pub to_recipients: Vec<String>,
    #[serde(default)]
    pub cc_recipients: Vec<String>,
    #[serde(default)]
    pub bcc_recipients: Vec<String>,
    #[serde(default)]
    pub unsafe_confirmed: bool,
}

pub async fn get_conversations(
    State(state): State<AppState>,
    Query(query): Query<MessagesQuery>,
    headers: HeaderMap,
) -> Result<Json<ConversationsResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    let conversations = store
        .list_portal_conversations(message_limit(&query))
        .await
        .map_err(ApiError::from_storage)?;
    Ok(Json(ConversationsResponse { conversations }))
}

pub async fn get_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<ConversationResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    let conversation = load_conversation(&store, conversation_id).await?;
    let conversation = with_reply_target(conversation);
    Ok(Json(ConversationResponse { conversation }))
}

pub async fn send_portal_message(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<PortalSendRequest>,
) -> Result<Json<ConversationResponse>, ApiError> {
    require_auth(&state, &headers)?;
    let config = state.config.read().expect("config lock poisoned").clone();
    let store = PgStore::connect(&config.database)
        .await
        .map_err(ApiError::from_storage)?;
    let detail = load_conversation(&store, conversation_id).await?;
    if detail.conversation.revision != request.thread_revision {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: "conversation changed; refresh before sending".to_string(),
        });
    }
    let mailbox = mailbox_for_conversation(&config, &detail.conversation)?;
    let prepared = prepare_portal_send(&store, mailbox, &detail, &request).await?;
    let (record, inserted) = store
        .begin_portal_message(&prepared.message)
        .await
        .map_err(ApiError::from_storage)?;
    if inserted {
        let (status, error) = match state
            .mail
            .send_composed(&mailbox.smtp, &prepared.email)
            .await
        {
            Ok(()) => ("sent", None),
            Err(error) => ("uncertain", Some(error.to_string())),
        };
        store
            .finish_portal_message(
                record.conversation_id,
                request.request_id,
                status,
                error.as_deref(),
            )
            .await
            .map_err(ApiError::from_storage)?;
        if prepared.source_conversation_to_touch != Some(record.conversation_id) {
            if let Some(source) = prepared.source_conversation_to_touch {
                store
                    .bump_conversation_revision(source)
                    .await
                    .map_err(ApiError::from_storage)?;
            }
        }
    }
    let conversation = load_conversation(&store, record.conversation_id).await?;
    Ok(Json(ConversationResponse {
        conversation: with_reply_target(conversation),
    }))
}

struct PreparedPortalSend {
    message: NewPortalMessage,
    email: ComposedEmail,
    source_conversation_to_touch: Option<Uuid>,
}

async fn prepare_portal_send(
    store: &PgStore,
    mailbox: &MailboxConfig,
    detail: &PortalConversationDetail,
    request: &PortalSendRequest,
) -> Result<PreparedPortalSend, ApiError> {
    let authored = authored_parts(request)?;
    match request.action {
        PortalSendAction::Reply => prepare_reply(mailbox, detail, request, authored),
        PortalSendAction::Forward => {
            prepare_forward(store, mailbox, detail, request, authored).await
        }
    }
}

fn prepare_reply(
    mailbox: &MailboxConfig,
    detail: &PortalConversationDetail,
    request: &PortalSendRequest,
    authored: AuthoredParts,
) -> Result<PreparedPortalSend, ApiError> {
    if detail.conversation.unsafe_reply_requires_confirmation && !request.unsafe_confirmed {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "unsafe conversation reply requires confirmation".to_string(),
        });
    }
    let reply_target = detail
        .conversation
        .remote_reply_to
        .as_deref()
        .map(reply_recipient)
        .filter(|target| !target.trim().is_empty())
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "conversation has no remote sender to reply to".to_string(),
        })?;
    let parent = latest_reply_parent(detail).ok_or_else(|| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "conversation has no message id to reply to".to_string(),
    })?;
    let message_id = outbound_message_id(mailbox);
    let references = references_for_parent(parent);
    let subject = reply_subject(&detail.conversation.subject);
    let rendered = render_with_quote(&authored, &detail.quote_text, &detail.quote_html);
    let email = ComposedEmail {
        to: vec![reply_target.clone()],
        cc: vec![],
        bcc: vec![],
        subject: subject.clone(),
        text_body: rendered.text.clone(),
        html_body: rendered.html.clone(),
        message_id: Some(message_id.clone()),
        in_reply_to: parent.message_id.clone(),
        references: references.clone(),
    };
    validate_portal_email(&email)?;
    Ok(PreparedPortalSend {
        email,
        source_conversation_to_touch: Some(detail.conversation.conversation_id),
        message: NewPortalMessage {
            portal_message_id: Uuid::new_v4(),
            conversation_id: detail.conversation.conversation_id,
            request_id: request.request_id,
            mailbox_id: detail.conversation.mailbox_id.clone(),
            thread_id: detail.conversation.thread_id.clone(),
            action: "reply".to_string(),
            to_recipients: vec![reply_target.clone()],
            cc_recipients: vec![],
            bcc_recipients: vec![],
            subject,
            authored_text: authored.text,
            authored_html: authored.html,
            rendered_text: rendered.text,
            rendered_html: rendered.html,
            quoted_text: detail.quote_text.clone(),
            quoted_html: Some(detail.quote_html.clone()),
            message_id,
            in_reply_to: parent.message_id.clone(),
            references,
            reply_target: Some(reply_target),
            source_conversation_id: None,
            child_conversation_id: None,
            unsafe_confirmed: request.unsafe_confirmed,
        },
    })
}

async fn prepare_forward(
    store: &PgStore,
    mailbox: &MailboxConfig,
    detail: &PortalConversationDetail,
    request: &PortalSendRequest,
    authored: AuthoredParts,
) -> Result<PreparedPortalSend, ApiError> {
    if request.to_recipients.is_empty()
        && request.cc_recipients.is_empty()
        && request.bcc_recipients.is_empty()
    {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "forward requires at least one recipient".to_string(),
        });
    }
    let generated_message_id = outbound_message_id(mailbox);
    let child_conversation_id = request.request_id;
    let subject = forward_subject(&detail.conversation.subject);
    let rendered = render_with_quote(&authored, &detail.quote_text, &detail.quote_html);
    let mut email = ComposedEmail {
        to: request.to_recipients.clone(),
        cc: request.cc_recipients.clone(),
        bcc: request.bcc_recipients.clone(),
        subject: subject.clone(),
        text_body: rendered.text.clone(),
        html_body: rendered.html.clone(),
        message_id: Some(generated_message_id.clone()),
        in_reply_to: None,
        references: vec![],
    };
    validate_portal_email(&email)?;
    let message_id = store
        .create_child_conversation(
            child_conversation_id,
            detail.conversation.conversation_id,
            &detail.conversation.mailbox_id,
            &generated_message_id,
            &subject,
        )
        .await
        .map_err(ApiError::from_storage)?;
    email.message_id = Some(message_id.clone());
    Ok(PreparedPortalSend {
        email,
        source_conversation_to_touch: Some(detail.conversation.conversation_id),
        message: NewPortalMessage {
            portal_message_id: Uuid::new_v4(),
            conversation_id: child_conversation_id,
            request_id: request.request_id,
            mailbox_id: detail.conversation.mailbox_id.clone(),
            thread_id: message_id.clone(),
            action: "forward".to_string(),
            to_recipients: request.to_recipients.clone(),
            cc_recipients: request.cc_recipients.clone(),
            bcc_recipients: request.bcc_recipients.clone(),
            subject,
            authored_text: authored.text,
            authored_html: authored.html,
            rendered_text: rendered.text,
            rendered_html: rendered.html,
            quoted_text: detail.quote_text.clone(),
            quoted_html: Some(detail.quote_html.clone()),
            message_id,
            in_reply_to: None,
            references: vec![],
            reply_target: None,
            source_conversation_id: Some(detail.conversation.conversation_id),
            child_conversation_id: Some(child_conversation_id),
            unsafe_confirmed: false,
        },
    })
}

struct AuthoredParts {
    text: String,
    html: Option<String>,
}

struct RenderedParts {
    text: String,
    html: Option<String>,
}

fn authored_parts(request: &PortalSendRequest) -> Result<AuthoredParts, ApiError> {
    let sanitized = request.authored_html.as_deref().map(sanitize_email_html);
    let text = if request.authored_text.trim().is_empty() {
        sanitized
            .as_ref()
            .map(|html| html.text.clone())
            .unwrap_or_default()
    } else {
        request.authored_text.trim_end().to_string()
    };
    if text.trim().is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "message body is required".to_string(),
        });
    }
    let html = sanitized
        .filter(|html| !html.visually_empty)
        .map(|html| html.html);
    Ok(AuthoredParts { text, html })
}

fn validate_portal_email(email: &ComposedEmail) -> Result<(), ApiError> {
    validate_composed_email(email).map_err(|errors| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: validation_message(&errors),
    })
}

fn validation_message(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|error| format!("{}: {}", error.field, error.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_with_quote(
    authored: &AuthoredParts,
    quote_text: &str,
    quote_html: &str,
) -> RenderedParts {
    let text = format!(
        "{}\n\n---------- Conversation history ---------\n{}",
        authored.text, quote_text
    );
    let html = authored.html.as_ref().map(|html| {
        format!("{html}<hr><blockquote data-ai-memmail-quote=\"history\">{quote_html}</blockquote>")
    });
    RenderedParts { text, html }
}

fn latest_reply_parent(detail: &PortalConversationDetail) -> Option<&PortalTimelineMessage> {
    detail.messages.iter().rev().find(|message| {
        reply_parent_belongs_to_conversation(detail, message) && message_has_id(message)
    })
}

fn reply_parent_belongs_to_conversation(
    detail: &PortalConversationDetail,
    message: &PortalTimelineMessage,
) -> bool {
    detail.conversation.source_conversation_id.is_some() || message.kind != "portal_forward"
}

fn message_has_id(message: &PortalTimelineMessage) -> bool {
    message
        .message_id
        .as_ref()
        .is_some_and(|id| !id.trim().is_empty())
}

fn references_for_parent(parent: &PortalTimelineMessage) -> Vec<String> {
    let mut references = parent.references.clone();
    if let Some(message_id) = &parent.message_id {
        if !references.iter().any(|reference| reference == message_id) {
            references.push(message_id.clone());
        }
    }
    references
}

fn reply_subject(subject: &str) -> String {
    if subject.trim_start().to_ascii_lowercase().starts_with("re:") {
        subject.trim().to_string()
    } else {
        format!("Re: {}", subject.trim())
    }
}

fn forward_subject(subject: &str) -> String {
    if subject
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("fwd:")
    {
        subject.trim().to_string()
    } else {
        format!("Fwd: {}", subject.trim())
    }
}

fn mailbox_for_conversation<'a>(
    config: &'a AppConfig,
    conversation: &PortalConversationSummary,
) -> Result<&'a MailboxConfig, ApiError> {
    config
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.id == conversation.mailbox_id)
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("mailbox {} is not configured", conversation.mailbox_id),
        })
}

async fn load_conversation(
    store: &PgStore,
    conversation_id: Uuid,
) -> Result<PortalConversationDetail, ApiError> {
    store
        .portal_conversation_detail(conversation_id)
        .await
        .map_err(ApiError::from_storage)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "conversation not found".to_string(),
        })
}

fn with_reply_target(mut detail: PortalConversationDetail) -> PortalConversationDetail {
    detail.conversation.remote_reply_to = detail
        .conversation
        .remote_reply_to
        .as_deref()
        .map(reply_recipient);
    detail
}

#[cfg(test)]
mod portal_tests {
    use super::*;

    #[test]
    fn source_reply_parent_skips_child_forward_message() {
        let inbound = timeline_message("inbound", "<inbound@example.com>");
        let forward = timeline_message("portal_forward", "<forward@example.com>");
        let detail = conversation_detail(None, vec![inbound.clone(), forward]);

        let parent = latest_reply_parent(&detail).unwrap();

        assert_eq!(parent.message_id, inbound.message_id);
    }

    #[test]
    fn child_reply_parent_can_use_child_forward_message() {
        let forward = timeline_message("portal_forward", "<forward@example.com>");
        let detail = conversation_detail(Some(Uuid::new_v4()), vec![forward.clone()]);

        let parent = latest_reply_parent(&detail).unwrap();

        assert_eq!(parent.message_id, forward.message_id);
    }

    fn conversation_detail(
        source_conversation_id: Option<Uuid>,
        messages: Vec<PortalTimelineMessage>,
    ) -> PortalConversationDetail {
        PortalConversationDetail {
            conversation: PortalConversationSummary {
                conversation_id: Uuid::new_v4(),
                mailbox_id: "support".to_string(),
                thread_id: "<thread@example.com>".to_string(),
                subject: "Question".to_string(),
                revision: 1,
                last_message_at: "2026-07-19T00:00:00Z".to_string(),
                latest_sender: "person@example.com".to_string(),
                latest_status: "open".to_string(),
                remote_reply_to: Some("person@example.com".to_string()),
                unsafe_reply_requires_confirmation: false,
                source_conversation_id,
                handoff: None,
            },
            messages,
            quote_text: String::new(),
            quote_html: String::new(),
        }
    }

    fn timeline_message(kind: &str, message_id: &str) -> PortalTimelineMessage {
        PortalTimelineMessage {
            id: format!("test:{message_id}"),
            direction: "outbound".to_string(),
            kind: kind.to_string(),
            status: "sent".to_string(),
            from_addr: "support@example.com".to_string(),
            to_recipients: vec!["person@example.com".to_string()],
            cc_recipients: vec![],
            bcc_recipients: vec![],
            subject: "Question".to_string(),
            text_body: Some("Body".to_string()),
            html_body: None,
            body_truncated: false,
            message_id: Some(message_id.to_string()),
            in_reply_to: None,
            references: vec![],
            safety_category: None,
            created_at: "2026-07-19T00:00:00Z".to_string(),
        }
    }
}
