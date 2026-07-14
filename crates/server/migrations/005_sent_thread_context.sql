ALTER TABLE processing_runs
    ADD COLUMN IF NOT EXISTS inbound_recipients TEXT[] NOT NULL DEFAULT '{}';

CREATE TABLE IF NOT EXISTS sent_messages (
    mailbox_id TEXT NOT NULL,
    folder_name TEXT NOT NULL,
    uid_validity BIGINT NOT NULL,
    uid BIGINT NOT NULL,
    thread_id TEXT NOT NULL,
    message_id TEXT,
    in_reply_to TEXT,
    message_references TEXT[] NOT NULL DEFAULT '{}',
    from_addr TEXT NOT NULL,
    recipients TEXT[] NOT NULL DEFAULT '{}',
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    body_truncated BOOLEAN NOT NULL DEFAULT FALSE,
    internal_date_epoch BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (mailbox_id, folder_name, uid_validity, uid)
);

CREATE TABLE IF NOT EXISTS mailbox_sync_state (
    mailbox_id TEXT NOT NULL,
    folder_role TEXT NOT NULL CHECK (folder_role IN ('sent')),
    folder_name TEXT NOT NULL,
    uid_validity BIGINT NOT NULL,
    last_uid BIGINT NOT NULL DEFAULT 0,
    backfill_cutoff_epoch BIGINT NOT NULL,
    initial_backfill_complete BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (mailbox_id, folder_role)
);

CREATE INDEX IF NOT EXISTS sent_messages_thread_idx
    ON sent_messages (mailbox_id, thread_id, internal_date_epoch, uid);
CREATE INDEX IF NOT EXISTS sent_messages_message_id_idx
    ON sent_messages (mailbox_id, message_id);
