impl PgStore {
    pub async fn upsert_email_conversation(
        &self,
        mailbox_id: &str,
        thread_id: &str,
        subject: &str,
    ) -> Result<uuid::Uuid, StorageError> {
        let row = self
            .client
            .query_one(
                "INSERT INTO email_conversations
                (conversation_id, mailbox_id, thread_id, subject, last_message_at)
                VALUES (md5($1 || ':' || $2)::uuid, $1, $2, $3, now())
                ON CONFLICT (mailbox_id, thread_id) DO UPDATE
                SET subject = CASE
                        WHEN EXCLUDED.subject <> '' THEN EXCLUDED.subject
                        ELSE email_conversations.subject
                    END,
                    updated_at = now()
                RETURNING conversation_id",
                &[&mailbox_id, &thread_id, &subject],
            )
            .await?;
        Ok(row.get(0))
    }

    pub async fn create_child_conversation(
        &self,
        conversation_id: uuid::Uuid,
        source_conversation_id: uuid::Uuid,
        mailbox_id: &str,
        thread_id: &str,
        subject: &str,
    ) -> Result<String, StorageError> {
        let row = self
            .client
            .query_one(
                "WITH inserted AS (
                    INSERT INTO email_conversations
                        (conversation_id, mailbox_id, thread_id, source_conversation_id, subject, last_message_at)
                    VALUES ($1, $2, $3, $4, $5, now())
                    ON CONFLICT DO NOTHING
                    RETURNING thread_id
                )
                SELECT thread_id FROM inserted
                UNION ALL
                SELECT thread_id
                FROM email_conversations
                WHERE conversation_id = $1
                LIMIT 1",
                &[
                    &conversation_id,
                    &mailbox_id,
                    &thread_id,
                    &source_conversation_id,
                    &subject,
                ],
            )
            .await?;
        Ok(row.get(0))
    }

    pub async fn list_portal_conversations(
        &self,
        limit: i64,
    ) -> Result<Vec<PortalConversationSummary>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT c.conversation_id, c.mailbox_id, c.thread_id, c.subject,
                    c.revision, c.last_message_at::text,
                    COALESCE(latest.from_addr, c.mailbox_id),
                    COALESCE(latest.status, 'open'),
                    remote.from_addr,
                    EXISTS (
                        SELECT 1
                        FROM processing_runs unsafe_pr
                        WHERE unsafe_pr.mailbox_id = c.mailbox_id
                            AND unsafe_pr.thread_id = c.thread_id
                            AND (
                                unsafe_pr.status = 'quarantined'
                                OR (
                                    unsafe_pr.safety_category IS NOT NULL
                                    AND unsafe_pr.safety_category <> 'safe'
                                )
                            )
                    ),
                    c.source_conversation_id,
                    th.state, th.destination, th.remote_target, th.last_error, th.updated_at::text
                FROM email_conversations c
                LEFT JOIN LATERAL (
                    SELECT from_addr, status
                    FROM processing_runs pr
                    WHERE pr.mailbox_id = c.mailbox_id AND pr.thread_id = c.thread_id
                    ORDER BY pr.updated_at DESC
                    LIMIT 1
                ) latest ON TRUE
                LEFT JOIN LATERAL (
                    SELECT from_addr
                    FROM processing_runs pr
                    WHERE pr.mailbox_id = c.mailbox_id AND pr.thread_id = c.thread_id
                    ORDER BY pr.created_at DESC
                    LIMIT 1
                ) remote ON TRUE
                LEFT JOIN thread_handoffs th
                    ON th.mailbox_id = c.mailbox_id AND th.thread_id = c.thread_id
                ORDER BY c.last_message_at DESC
                LIMIT $1",
                &[&limit],
            )
            .await?;
        Ok(rows.into_iter().map(portal_conversation_from_row).collect())
    }

    pub async fn portal_conversation_detail(
        &self,
        conversation_id: uuid::Uuid,
    ) -> Result<Option<PortalConversationDetail>, StorageError> {
        let Some(conversation) = self.portal_conversation_summary(conversation_id).await? else {
            return Ok(None);
        };
        let mut messages = self
            .portal_processing_messages(&conversation.mailbox_id, &conversation.thread_id)
            .await?;
        messages.extend(self.portal_authored_messages(&conversation).await?);
        messages.sort_by(|first, second| first.created_at.cmp(&second.created_at));
        let quote_text = portal_quote_text(&messages);
        let quote_html = portal_quote_html(&messages);
        Ok(Some(PortalConversationDetail {
            conversation,
            messages,
            quote_text,
            quote_html,
        }))
    }

    pub async fn begin_portal_message(
        &self,
        message: &NewPortalMessage,
    ) -> Result<(PortalMessageRecord, bool), StorageError> {
        let inserted = self
            .client
            .query_opt(
                "INSERT INTO portal_messages
                (portal_message_id, conversation_id, request_id, mailbox_id, thread_id, action,
                    status, to_recipients, cc_recipients, bcc_recipients, subject,
                    authored_text, authored_html, rendered_text, rendered_html,
                    quoted_text, quoted_html, message_id, in_reply_to, message_references,
                    reply_target, source_conversation_id, child_conversation_id, unsafe_confirmed)
                VALUES ($1, $2, $3, $4, $5, $6, 'sending', $7, $8, $9, $10,
                    $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
                ON CONFLICT (request_id) DO NOTHING
                RETURNING portal_message_id, conversation_id, request_id, action, status,
                    to_recipients, cc_recipients, bcc_recipients, subject, authored_text,
                    authored_html, rendered_text, rendered_html, quoted_text, quoted_html,
                    message_id, in_reply_to, message_references, reply_target,
                    child_conversation_id, error",
                &[
                    &message.portal_message_id,
                    &message.conversation_id,
                    &message.request_id,
                    &message.mailbox_id,
                    &message.thread_id,
                    &message.action,
                    &message.to_recipients,
                    &message.cc_recipients,
                    &message.bcc_recipients,
                    &message.subject,
                    &message.authored_text,
                    &message.authored_html,
                    &message.rendered_text,
                    &message.rendered_html,
                    &message.quoted_text,
                    &message.quoted_html,
                    &message.message_id,
                    &message.in_reply_to,
                    &message.references,
                    &message.reply_target,
                    &message.source_conversation_id,
                    &message.child_conversation_id,
                    &message.unsafe_confirmed,
                ],
            )
            .await?;
        if let Some(row) = inserted {
            self.bump_conversation_revision(message.conversation_id).await?;
            return Ok((portal_message_from_row(row), true));
        }
        let existing = self.portal_message_by_request(message.request_id).await?;
        Ok((existing, false))
    }

    pub async fn finish_portal_message(
        &self,
        conversation_id: uuid::Uuid,
        request_id: uuid::Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "UPDATE portal_messages
                SET status = $1, error = $2, updated_at = now()
                WHERE conversation_id = $3 AND request_id = $4",
                &[&status, &error, &conversation_id, &request_id],
            )
            .await?;
        self.bump_conversation_revision(conversation_id).await
    }

    async fn portal_conversation_summary(
        &self,
        conversation_id: uuid::Uuid,
    ) -> Result<Option<PortalConversationSummary>, StorageError> {
        let row = self
            .client
            .query_opt(
                "SELECT c.conversation_id, c.mailbox_id, c.thread_id, c.subject,
                    c.revision, c.last_message_at::text,
                    COALESCE(latest.from_addr, c.mailbox_id),
                    COALESCE(latest.status, 'open'),
                    remote.from_addr,
                    EXISTS (
                        SELECT 1
                        FROM processing_runs unsafe_pr
                        WHERE unsafe_pr.mailbox_id = c.mailbox_id
                            AND unsafe_pr.thread_id = c.thread_id
                            AND (
                                unsafe_pr.status = 'quarantined'
                                OR (
                                    unsafe_pr.safety_category IS NOT NULL
                                    AND unsafe_pr.safety_category <> 'safe'
                                )
                            )
                    ),
                    c.source_conversation_id,
                    th.state, th.destination, th.remote_target, th.last_error, th.updated_at::text
                FROM email_conversations c
                LEFT JOIN LATERAL (
                    SELECT from_addr, status
                    FROM processing_runs pr
                    WHERE pr.mailbox_id = c.mailbox_id AND pr.thread_id = c.thread_id
                    ORDER BY pr.updated_at DESC
                    LIMIT 1
                ) latest ON TRUE
                LEFT JOIN LATERAL (
                    SELECT from_addr
                    FROM processing_runs pr
                    WHERE pr.mailbox_id = c.mailbox_id AND pr.thread_id = c.thread_id
                    ORDER BY pr.created_at DESC
                    LIMIT 1
                ) remote ON TRUE
                LEFT JOIN thread_handoffs th
                    ON th.mailbox_id = c.mailbox_id AND th.thread_id = c.thread_id
                WHERE c.conversation_id = $1",
                &[&conversation_id],
            )
            .await?;
        Ok(row.map(portal_conversation_from_row))
    }

    async fn portal_message_by_request(
        &self,
        request_id: uuid::Uuid,
    ) -> Result<PortalMessageRecord, StorageError> {
        let row = self
            .client
            .query_one(
                "SELECT portal_message_id, conversation_id, request_id, action, status,
                    to_recipients, cc_recipients, bcc_recipients, subject, authored_text,
                    authored_html, rendered_text, rendered_html, quoted_text, quoted_html,
                    message_id, in_reply_to, message_references, reply_target,
                    child_conversation_id, error
                FROM portal_messages
                WHERE request_id = $1",
                &[&request_id],
            )
            .await?;
        Ok(portal_message_from_row(row))
    }

    async fn portal_processing_messages(
        &self,
        mailbox_id: &str,
        thread_id: &str,
    ) -> Result<Vec<PortalTimelineMessage>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT run_id::text, message_id, in_reply_to, message_references, from_addr,
                    inbound_recipients, subject, inbound_body, inbound_body_truncated,
                    status, safety_category, created_at::text,
                    outbound_message_id, outbound_recipients, outbound_subject, outbound_body,
                    outbound_body_html, outbound_body_redacted, outbound_action, updated_at::text
                FROM processing_runs
                WHERE mailbox_id = $1 AND thread_id = $2",
                &[&mailbox_id, &thread_id],
            )
            .await?;
        let mut messages = Vec::new();
        for row in rows {
            if let Some(body) = row.get::<_, Option<String>>(7) {
                messages.push(PortalTimelineMessage {
                    id: format!("inbound:{}", row.get::<_, String>(0)),
                    direction: "inbound".to_string(),
                    kind: "inbound".to_string(),
                    status: row.get(9),
                    from_addr: row.get(4),
                    to_recipients: row.get(5),
                    cc_recipients: vec![],
                    bcc_recipients: vec![],
                    subject: row.get(6),
                    text_body: Some(body),
                    html_body: None,
                    body_truncated: row.get(8),
                    message_id: row.get(1),
                    in_reply_to: row.get(2),
                    references: row.get(3),
                    safety_category: row.get(10),
                    created_at: row.get(11),
                });
            }
            if row.get::<_, Option<String>>(18).as_deref() == Some("reply") {
                if let Some(body) = row.get::<_, Option<String>>(15) {
                    messages.push(PortalTimelineMessage {
                        id: format!("ai:{}", row.get::<_, String>(0)),
                        direction: "outbound".to_string(),
                        kind: "ai_reply".to_string(),
                        status: row.get(9),
                        from_addr: mailbox_id.to_string(),
                        to_recipients: row.get(13),
                        cc_recipients: vec![],
                        bcc_recipients: vec![],
                        subject: row.get::<_, Option<String>>(14).unwrap_or_default(),
                        text_body: Some(body),
                        html_body: row.get(16),
                        body_truncated: row.get(17),
                        message_id: row.get(12),
                        in_reply_to: row.get(1),
                        references: {
                            let mut refs = row.get::<_, Vec<String>>(3);
                            if let Some(id) = row.get::<_, Option<String>>(1) {
                                refs.push(id);
                            }
                            refs
                        },
                        safety_category: None,
                        created_at: row.get(19),
                    });
                }
            }
        }
        Ok(messages)
    }

    async fn portal_authored_messages(
        &self,
        conversation: &PortalConversationSummary,
    ) -> Result<Vec<PortalTimelineMessage>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT portal_message_id::text, action, status, to_recipients, cc_recipients,
                    bcc_recipients, subject, authored_text, authored_html, message_id,
                    in_reply_to, message_references, created_at::text
                FROM portal_messages
                WHERE mailbox_id = $1 AND thread_id = $2
                ORDER BY created_at ASC",
                &[&conversation.mailbox_id, &conversation.thread_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| PortalTimelineMessage {
                id: format!("portal:{}", row.get::<_, String>(0)),
                direction: "outbound".to_string(),
                kind: format!("portal_{}", row.get::<_, String>(1)),
                status: row.get(2),
                from_addr: conversation.mailbox_id.clone(),
                to_recipients: row.get(3),
                cc_recipients: row.get(4),
                bcc_recipients: row.get(5),
                subject: row.get(6),
                text_body: Some(row.get(7)),
                html_body: row.get(8),
                body_truncated: false,
                message_id: Some(row.get(9)),
                in_reply_to: row.get(10),
                references: row.get(11),
                safety_category: None,
                created_at: row.get(12),
            })
            .collect())
    }

    pub async fn bump_conversation_revision(
        &self,
        conversation_id: uuid::Uuid,
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "UPDATE email_conversations
                SET revision = revision + 1, last_message_at = now(), updated_at = now()
                WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .await?;
        Ok(())
    }
}

fn portal_quote_text(messages: &[PortalTimelineMessage]) -> String {
    let mut quote = String::new();
    for (index, message) in messages.iter().enumerate() {
        let body = message
            .text_body
            .as_deref()
            .unwrap_or("[message body unavailable]");
        if !quote.is_empty() {
            quote.push_str("\n\n");
        }
        quote.push_str(&format!(
            "[{}] {}\nFrom: {}\nTo: {}\nSubject: {}\nMessage-ID: {}\n\n{}",
            index + 1,
            message.kind,
            message.from_addr,
            if message.to_recipients.is_empty() {
                "(none)".to_string()
            } else {
                message.to_recipients.join(", ")
            },
            message.subject,
            message.message_id.as_deref().unwrap_or("(none)"),
            if message.body_truncated {
                format!("{body}\n\n[Stored body was truncated by ai-memmail.]")
            } else {
                body.to_string()
            }
        ));
    }
    quote
}

fn portal_quote_html(messages: &[PortalTimelineMessage]) -> String {
    let mut html = String::new();
    for message in messages {
        html.push_str("<section class=\"quoted-message\">");
        html.push_str("<p><strong>");
        html.push_str(&escape_html(&message.kind));
        html.push_str("</strong><br>From: ");
        html.push_str(&escape_html(&message.from_addr));
        html.push_str("<br>To: ");
        html.push_str(&escape_html(&message.to_recipients.join(", ")));
        html.push_str("<br>Subject: ");
        html.push_str(&escape_html(&message.subject));
        html.push_str("</p>");
        if let Some(body) = &message.html_body {
            html.push_str(body);
        } else if let Some(body) = &message.text_body {
            html.push_str("<pre>");
            html.push_str(&escape_html(body));
            html.push_str("</pre>");
        }
        if message.body_truncated {
            html.push_str("<p>[Stored body was truncated by ai-memmail.]</p>");
        }
        html.push_str("</section>");
    }
    html
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn portal_conversation_from_row(row: tokio_postgres::Row) -> PortalConversationSummary {
    PortalConversationSummary {
        conversation_id: row.get(0),
        mailbox_id: row.get(1),
        thread_id: row.get(2),
        subject: row.get(3),
        revision: row.get(4),
        last_message_at: row.get(5),
        latest_sender: row.get(6),
        latest_status: row.get(7),
        remote_reply_to: row.get(8),
        unsafe_reply_requires_confirmation: row.get(9),
        source_conversation_id: row.get(10),
        handoff: row.get::<_, Option<String>>(11).map(|state| ThreadHandoffSummary {
            state,
            destination: row.get(12),
            remote_target: row.get(13),
            last_error: row.get(14),
            updated_at: row.get(15),
        }),
    }
}

fn portal_message_from_row(row: tokio_postgres::Row) -> PortalMessageRecord {
    PortalMessageRecord {
        portal_message_id: row.get(0),
        conversation_id: row.get(1),
        request_id: row.get(2),
        action: row.get(3),
        status: row.get(4),
        to_recipients: row.get(5),
        cc_recipients: row.get(6),
        bcc_recipients: row.get(7),
        subject: row.get(8),
        authored_text: row.get(9),
        authored_html: row.get(10),
        rendered_text: row.get(11),
        rendered_html: row.get(12),
        quoted_text: row.get(13),
        quoted_html: row.get(14),
        message_id: row.get(15),
        in_reply_to: row.get(16),
        references: row.get(17),
        reply_target: row.get(18),
        child_conversation_id: row.get(19),
        error: row.get(20),
    }
}
