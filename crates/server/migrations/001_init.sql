CREATE TABLE IF NOT EXISTS processing_runs (
    run_id UUID PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    uid_validity BIGINT NOT NULL,
    uid BIGINT NOT NULL,
    message_id TEXT,
    from_addr TEXT NOT NULL,
    subject TEXT NOT NULL,
    status TEXT NOT NULL,
    safety_category TEXT,
    safety_reason TEXT,
    outbound_action TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (mailbox_id, uid_validity, uid)
);

ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS agent_action TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS agent_safety_notes TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_recipients TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_subject TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_body TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_body_redacted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_reason TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_review_status TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_review_reason TEXT;

CREATE TABLE IF NOT EXISTS sender_reviews (
    sender TEXT PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS banned_senders (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('email', 'domain')),
    value TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (kind, value)
);

CREATE TABLE IF NOT EXISTS action_logs (
    id BIGSERIAL PRIMARY KEY,
    level TEXT NOT NULL CHECK (level IN ('debug', 'info', 'warn', 'error', 'fatal')),
    run_id TEXT NOT NULL,
    mailbox_id TEXT,
    message_uid_validity BIGINT,
    message_uid BIGINT,
    action TEXT NOT NULL,
    status TEXT NOT NULL,
    duration_ms BIGINT NOT NULL,
    detail TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE action_logs ADD COLUMN IF NOT EXISTS message_uid_validity BIGINT;

CREATE INDEX IF NOT EXISTS action_logs_created_at_idx ON action_logs (created_at DESC);
CREATE INDEX IF NOT EXISTS action_logs_mailbox_idx ON action_logs (mailbox_id, created_at DESC);
CREATE INDEX IF NOT EXISTS processing_runs_status_idx ON processing_runs (status, created_at DESC);
