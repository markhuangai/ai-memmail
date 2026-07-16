CREATE TABLE IF NOT EXISTS thread_handoffs (
    mailbox_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    destination TEXT NOT NULL,
    remote_target TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('sending', 'active', 'uncertain')),
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (mailbox_id, thread_id)
);

CREATE TABLE IF NOT EXISTS thread_handoff_deliveries (
    id BIGSERIAL PRIMARY KEY,
    request_id UUID NOT NULL,
    mailbox_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source_run_id UUID,
    destination TEXT NOT NULL,
    remote_target TEXT NOT NULL,
    outbound_message_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('sending', 'sent', 'failed', 'uncertain')),
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (mailbox_id, thread_id, request_id)
);

CREATE INDEX IF NOT EXISTS thread_handoffs_updated_at_idx
    ON thread_handoffs (updated_at DESC);
CREATE INDEX IF NOT EXISTS thread_handoff_deliveries_thread_idx
    ON thread_handoff_deliveries (mailbox_id, thread_id, created_at DESC);
CREATE INDEX IF NOT EXISTS thread_handoff_deliveries_message_id_idx
    ON thread_handoff_deliveries (mailbox_id, outbound_message_id);
