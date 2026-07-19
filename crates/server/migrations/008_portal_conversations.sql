CREATE TABLE IF NOT EXISTS email_conversations (
    conversation_id UUID PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source_conversation_id UUID REFERENCES email_conversations(conversation_id),
    subject TEXT NOT NULL DEFAULT '',
    revision BIGINT NOT NULL DEFAULT 0,
    last_message_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (mailbox_id, thread_id)
);

INSERT INTO email_conversations
    (conversation_id, mailbox_id, thread_id, subject, last_message_at, created_at, updated_at)
SELECT DISTINCT ON (mailbox_id, resolved_thread_id)
    md5(mailbox_id || ':' || resolved_thread_id)::uuid,
    mailbox_id,
    resolved_thread_id,
    subject,
    updated_at,
    created_at,
    updated_at
FROM (
    SELECT
        mailbox_id,
        COALESCE(thread_id, message_id, mailbox_id || ':' || uid_validity::text || ':' || uid::text) AS resolved_thread_id,
        subject,
        created_at,
        updated_at
    FROM processing_runs
) resolved
ORDER BY mailbox_id, resolved_thread_id, updated_at DESC
ON CONFLICT (mailbox_id, thread_id) DO NOTHING;

CREATE TABLE IF NOT EXISTS portal_messages (
    portal_message_id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES email_conversations(conversation_id),
    request_id UUID NOT NULL,
    mailbox_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('reply', 'forward')),
    status TEXT NOT NULL CHECK (status IN ('sending', 'sent', 'uncertain')),
    to_recipients TEXT[] NOT NULL DEFAULT '{}',
    cc_recipients TEXT[] NOT NULL DEFAULT '{}',
    bcc_recipients TEXT[] NOT NULL DEFAULT '{}',
    subject TEXT NOT NULL,
    authored_text TEXT NOT NULL,
    authored_html TEXT,
    rendered_text TEXT NOT NULL,
    rendered_html TEXT,
    quoted_text TEXT NOT NULL DEFAULT '',
    quoted_html TEXT,
    message_id TEXT NOT NULL,
    in_reply_to TEXT,
    message_references TEXT[] NOT NULL DEFAULT '{}',
    reply_target TEXT,
    source_conversation_id UUID REFERENCES email_conversations(conversation_id),
    child_conversation_id UUID REFERENCES email_conversations(conversation_id),
    unsafe_confirmed BOOLEAN NOT NULL DEFAULT FALSE,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (conversation_id, request_id),
    UNIQUE (request_id)
);

CREATE INDEX IF NOT EXISTS email_conversations_updated_at_idx
    ON email_conversations (last_message_at DESC);
CREATE INDEX IF NOT EXISTS portal_messages_conversation_idx
    ON portal_messages (conversation_id, created_at ASC);
CREATE INDEX IF NOT EXISTS portal_messages_thread_idx
    ON portal_messages (mailbox_id, thread_id, created_at ASC);
CREATE INDEX IF NOT EXISTS portal_messages_message_id_idx
    ON portal_messages (mailbox_id, message_id);
