ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS thread_id TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS in_reply_to TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS message_references TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS inbound_body TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS inbound_body_truncated BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS outbound_message_id TEXT;

UPDATE processing_runs
SET thread_id = COALESCE(message_id, mailbox_id || ':' || uid_validity::text || ':' || uid::text)
WHERE thread_id IS NULL;

CREATE INDEX IF NOT EXISTS processing_runs_thread_id_idx ON processing_runs (thread_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS processing_runs_message_id_idx ON processing_runs (message_id);
CREATE INDEX IF NOT EXISTS processing_runs_outbound_message_id_idx ON processing_runs (outbound_message_id);
