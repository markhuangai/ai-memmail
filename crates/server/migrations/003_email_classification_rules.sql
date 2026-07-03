CREATE TABLE IF NOT EXISTS email_categories (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    source TEXT NOT NULL DEFAULT 'user' CHECK (source IN ('seed', 'user', 'ai')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS email_topics (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    source TEXT NOT NULL DEFAULT 'user' CHECK (source IN ('seed', 'user', 'ai')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS email_rules (
    id BIGSERIAL PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    name TEXT NOT NULL,
    category_id BIGINT NOT NULL REFERENCES email_categories(id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK (action IN ('reply', 'forward', 'noop')),
    reply_goal TEXT NOT NULL DEFAULT '',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    priority INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS email_rule_topics (
    rule_id BIGINT NOT NULL REFERENCES email_rules(id) ON DELETE CASCADE,
    topic_id BIGINT NOT NULL REFERENCES email_topics(id) ON DELETE CASCADE,
    PRIMARY KEY (rule_id, topic_id)
);

CREATE TABLE IF NOT EXISTS email_rule_mailbox_seeds (
    mailbox_id TEXT PRIMARY KEY,
    seeded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS classification_category_id BIGINT REFERENCES email_categories(id) ON DELETE SET NULL;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS classification_topic_ids BIGINT[] NOT NULL DEFAULT '{}';
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS classification_reason TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS classification_confidence SMALLINT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS decision_source TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS matched_rule_id BIGINT REFERENCES email_rules(id) ON DELETE SET NULL;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS matched_rule_name TEXT;
ALTER TABLE processing_runs ADD COLUMN IF NOT EXISTS matched_rule_goal TEXT;

INSERT INTO email_categories (name, description, source)
VALUES
    ('marketing_vendor', 'Paid marketing, growth, SEO, lead-generation, advertising, PR, agency, tool, or vendor service outreach.', 'seed'),
    ('greeting', 'Short hello, introduction, thanks, or relationship maintenance with no substantial ask.', 'seed'),
    ('question', 'A concrete question about Mark, a project, article, setup, usage, or technical direction.', 'seed'),
    ('project_opportunity', 'A collaboration, contribution, integration, partnership, investment, job, speaking, or project opportunity that may need Mark''s judgment.', 'seed'),
    ('other', 'Anything that does not clearly fit the other configured categories.', 'seed')
ON CONFLICT (name) DO UPDATE
SET description = EXCLUDED.description,
    status = 'active',
    updated_at = now();

INSERT INTO email_topics (name, description, source)
VALUES
    ('dense_mem', 'Dense-Mem: governed AI memory, MCP access, evidence, typed claims/facts, conflicts, recall, and team/profile isolation.', 'seed'),
    ('ai_memmail', 'ai-memmail: the email-processing agent, control panel, IMAP/SMTP workflow, history, prompts, and rules.', 'seed'),
    ('gitvibe', 'GitVibe: maintainer-gated AI development automation for GitHub issues, PRs, labels, workflows, and reviews.', 'seed'),
    ('agentool', 'agentool: production-ready Vercel AI SDK tools for agents, file operations, shell, search, memory, and context compaction.', 'seed'),
    ('ai_memory', 'AI memory, RAG limits, graph-backed recall, provenance, conflict handling, retrieval policy, and durable assistant context.', 'seed'),
    ('general', 'General or unclear topic.', 'seed')
ON CONFLICT (name) DO UPDATE
SET description = EXCLUDED.description,
    status = 'active',
    updated_at = now();

CREATE INDEX IF NOT EXISTS email_rules_mailbox_category_idx ON email_rules (mailbox_id, category_id, enabled, priority);
CREATE INDEX IF NOT EXISTS processing_runs_classification_idx ON processing_runs (classification_category_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS processing_runs_matched_rule_idx ON processing_runs (matched_rule_id, updated_at DESC);
