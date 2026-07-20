<h1 align="center">ai-memmail</h1>

<p align="center">
  <img src="https://img.shields.io/badge/ai--memmail-IMAP_MCP_email_agent-0f766e?style=for-the-badge&logo=github&logoColor=white" alt="ai-memmail" />
</p>

<p align="center">
  <strong>Email automation for AI agents that reads IMAP mail, recalls Dense-Mem context, and sends policy-checked SMTP replies.</strong>
</p>

<p align="center">
  <a href="mailto:contact@markhuang.ai?subject=ai-memmail%20demo%20question"><img src="https://img.shields.io/badge/Try%20ai--memmail%20live-Email%20hosted%20demo-0f766e?style=for-the-badge" alt="Try ai-memmail live" /></a>
</p>

<p align="center">
  <a href="https://github.com/markhuangai/ai-memmail"><img src="https://img.shields.io/github/stars/markhuangai/ai-memmail?style=flat-square&logo=github" alt="GitHub stars" /></a>
  <a href="https://github.com/markhuangai/ai-memmail/issues"><img src="https://img.shields.io/github/issues/markhuangai/ai-memmail?style=flat-square&logo=github" alt="GitHub issues" /></a>
  <a href="https://github.com/markhuangai/ai-memmail/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue?style=flat-square" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/Rust-1.80-000000?style=flat-square&logo=rust&logoColor=white" alt="Rust 1.80" />
  <a href="https://github.com/markhuangai/ai-memmail/pkgs/container/ai-memmail"><img src="https://img.shields.io/badge/Docker-GHCR-2496ED?style=flat-square&logo=docker&logoColor=white" alt="Docker image on GHCR" /></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/IMAP%2FSMTP-email-111827?style=flat-square" alt="IMAP and SMTP email" />
  <img src="https://img.shields.io/badge/MCP-Dense--Mem-0f766e?style=flat-square" alt="Dense-Mem MCP" />
  <img src="https://img.shields.io/badge/PostgreSQL-16-4169E1?style=flat-square&logo=postgresql&logoColor=white" alt="PostgreSQL 18" />
  <img src="https://img.shields.io/badge/React-TypeScript-61DAFB?style=flat-square&logo=react&logoColor=111827" alt="React TypeScript" />
  <img src="https://visitor-badge.laobi.icu/badge?page_id=markhuangai.ai-memmail&style=flat-square" alt="Visitors" />
</p>

ai-memmail is a Rust email agent that monitors IMAP mailboxes, uses configured
Dense-Mem MCP servers to answer mail when policy allows it, and sends replies or
forwards through SMTP. The control panel is a same-origin React TypeScript
dashboard served by the Rust web service.

## Live Hosted Demo

A live hosted ai-memmail demo is available at `contact@markhuang.ai`. Send an
email to that address with a question about Dense-Mem or this ai-memmail project
to test the IMAP, MCP-backed answer, and SMTP reply flow.

Do not send secrets, credentials, or sensitive personal data to the demo inbox.

## Current Status

The repository now contains the v1 application foundation:

- Rust workspace with an Axum control-panel API, IMAP polling worker,
  OpenAI-compatible safety/agent decisions, Dense-Mem MCP recall over HTTP,
  SMTP reply/forward sending, typed YAML configuration, prompt-file loading,
  safety policy primitives, email classification/rule policies, structured
  action logging, and PostgreSQL migrations.
- React TypeScript control panel for login, mailbox settings, MCP server
  settings, safety lists, classification rules, AI prompt paths, and logging
  settings.
- Source-built Docker runtime that runs the web API and worker side by side in
  one app process.
- Backend and frontend unit coverage gates, plus Playwright E2E coverage for the
  control panel.

PostgreSQL persistence tracks processing run state, processed-message history,
structured decisions, classification results, rule matches, outbound actions,
manual Sent messages, mailbox sync cursors, and action logs. The worker still
relies on IMAP `UNSEEN` plus `Seen` marking for source-mail delivery state.

## Local Setup

Create a local config and panel key:

```bash
cp config/config.example.yaml config/config.yaml
export CONTROL_PANEL_KEY="replace-with-local-key"
```

Run the full local stack with the published container image:

```bash
docker compose up
```

The control panel is served at `http://127.0.0.1:18080` by default. PostgreSQL
is exposed on `127.0.0.1:15432` by default to avoid conflicts with a host
Postgres install. To change either host port, edit the port forwarding in
`docker-compose.yml` or set `AI_MEMMAIL_HTTP_PORT`, for example:

```bash
AI_MEMMAIL_HTTP_PORT=18081 docker compose up
```

For live development with real credentials, edit the ignored
`config/config.yaml` file directly, then run:

```bash
scripts/live-e2e.sh
```

`scripts/live-e2e.sh` uses `CONTROL_PANEL_KEY=live-e2e-local` by default for
local testing. Override it when needed:

```bash
CONTROL_PANEL_KEY="replace-with-local-key" scripts/live-e2e.sh
```

If another local service already uses `18080`, run the live E2E on another
control-panel port:

```bash
AI_MEMMAIL_HTTP_PORT=18081 scripts/live-e2e.sh
```

The live mail test loads `config/config.yaml`, starts PostgreSQL, sends one
email scenario at a time, runs the real worker path from the test process, waits
for the expected reply or forward, then starts the app for the browser E2E
check. It covers MCP-backed known-answer reply, explicit human-forward routing,
prompt-injection quarantine forwarding, and banned-sender forwarding. The test
derives the forward mailbox credentials from the local config in memory; do not
commit local credentials.

## v1 Architecture

```text
IMAP Sent mailbox -> canonical outbound thread history
IMAP inbox
  -> worker fetches message metadata and body
  -> metadata is stored in PostgreSQL
  -> sender ban/review checks run
  -> safety scan runs with no MCP tools and no send capability
      -> flagged: quarantine, add sender to review, forward to human reviewer
      -> safe: classify category/topic labels
          -> matching rule: draft the configured action goal
          -> no rule: run mailbox agent with allowed MCP tools
  -> SMTP reply or forward
  -> action logs go to stdout and PostgreSQL
  -> control panel shows history, body, thread, labels, rule match, and logs
```

The production runtime uses one source-built image and starts both long-running
paths in one process:

- `web`: Axum API and React control panel.
- `worker`: IMAP polling, safety scanning, AI/MCP processing, and SMTP sending.

At process startup, `ai-memmail-server` connects to PostgreSQL and applies
versioned SQL migrations before starting the web API and worker. Migration
application uses a PostgreSQL advisory lock and records applied versions plus
checksums in `schema_migrations`, so concurrent replicas serialize schema
changes and detect edited historical migrations.

The web API and worker share PostgreSQL. Inbound plain-text bodies are stored in
processed-message history up to a bounded size so the control panel can explain
what was processed. Metadata, processing decisions, safety results,
classification labels, rule matches, outbound reply bodies, sender review
state, banned senders, and action logs are also persisted. Forward bodies are
redacted before storage because they can include full original inbound content.
Reprocessing refetches source messages from IMAP.

## Mail Flow

v1 uses password-authenticated IMAP/IMAPS for inbound mail and SMTP/SMTPS for
outbound mail. OAuth2 is intentionally out of scope for v1.

After successful processing, ai-memmail marks the source message as `Seen`.
Deduplication uses mailbox id plus IMAP `UIDVALIDITY` and UID so repeated polls
do not send duplicate replies.

### Thread context and size limits

Before processing unseen inbox mail, the worker synchronizes the mailbox's
manual Sent messages into PostgreSQL. It auto-discovers exactly one IMAP folder
advertised with the `\Sent` special-use attribute; set `imap.sent_folder` when
the server does not advertise one or advertises more than one. The first sync
backfills `imap.sent_backfill_days` (30 by default), then resumes by folder,
`UIDVALIDITY`, and UID. Inbox processing fails closed while this sync fails or
the initial backfill is incomplete so a reply is not generated without known
outbound history. Set the backfill to `0` only to disable Sent synchronization.

`Message-ID`, `In-Reply-To`, and `References` link manual Sent messages,
processed inbound messages, and application replies into one mailbox-scoped
canonical thread. The full ordered thread is included as untrusted AI input.
When canonical history is available, common client quote blocks are removed
from the current authored reply so the same prior text is not duplicated.

Stored bodies remain capped at 64 KiB. AI requests have a hard limit of
1,000,000 serialized characters. A required prior body that was truncated, a
locally oversized request, or a provider context-limit response immediately
forwards the current message to the configured human reviewer. The worker does
not summarize, drop required history, or retry with a smaller AI prompt.

## Untrusted Email Policy

Every inbound email is treated as untrusted data. Email bodies are never treated
as system instructions.

Processing order:

1. Fetch the message from IMAP.
2. Persist metadata and create a processing run.
3. Check the sender against the banned sender list.
4. Parse the message for scan input.
5. Run the safety scan using the configured prompt file. This scanner has no MCP
   tools and no send capability.
6. If the scan flags jailbreak, prompt injection, malicious hacking content, or
   sensitive exfiltration attempts:
   - mark the run as quarantined
   - add the sender to the review list
   - forward the original message inline to the mailbox-specific
     `safety_forward_to` address
   - prefix the forwarded subject with a potential-jailbreak warning
   - include the scan reason in the forward body
7. If safe, classify the email against active categories and topics. The AI sees
   the configured taxonomy and may create a new label only when the existing
   labels cannot honestly describe the email.
8. Match enabled mailbox rules by category, then optional topics. A rule with no
   topics matches any topic in its category. If a rule matches, the AI drafts
   the configured action goal and the application controls recipients,
   threading, and final action type.
9. If no rule matches, run the mailbox agent with only the mailbox's allowed MCP
   servers.
10. Optionally run a second outbound AI review pass. It exists in v1 but is
   disabled by default.
11. Send only if deterministic validation accepts the structured AI result.

Default classification labels are seeded in PostgreSQL. The initial categories
are `marketing_vendor`, `greeting`, `question`, `project_opportunity`, and
`other`; initial topics include `dense_mem`, `ai_memmail`, `gitvibe`,
`agentool`, `ai_memory`, and `general`. The worker/web API seeds one enabled
per-mailbox rule that auto-declines `marketing_vendor` outreach. Operators can
edit rules in the control panel without changing YAML or mail-server labels.

## Configuration

Configuration is YAML-only for v1, including secrets. This is simple and matches
the requested local workflow, but it is a production risk: mounted config files,
backups, logs, and accidental commits can expose credentials. Local credential
files must stay untracked.

Ignored local files include:

- `.env.local`
- `config/config.yaml`
- `config/local.yaml`
- `config/live.local.yaml`

System prompts are configured as file paths, not inline prompt text. Paths are
resolved relative to `prompts.root`.

Each `mailboxes[].id` is the stable operational id stored in logs and
processing metadata. Use a real production id such as `support` or an address
slug; do not leave copied sandbox values like `test` in production config.

Example shape:

```yaml
version: 1

database:
  host: postgres
  port: 5432
  username: ai_memmail
  password: ai_memmail
  database: ai_memmail

logging:
  level: info
  format: json
  verbose_actions: true
  retention_days: 180

prompts:
  root: "./prompts"
  safety_scan: "safety-scan.md"
  email_classifier: "email-classifier.md"
  rule_action: "rule-action.md"

ai:
  protocol: openai
  AI_API_URL: "https://api.example/v1"
  AI_API_SECRET: "replace-local-secret"
  AI_MODEL: "model-name"
  review:
    enabled: false
    prompt_path: "outbound-review.md"

mcp_servers:
  dense_mem_primary:
    transport: stdio
    command: npx
    args: ["-y", "dense-mem-mcp-proxy"]
    env:
      DENSE_MEM_MCP_URL: "http://dense-mem:8080/mcp"
      DENSE_MEM_API_KEY: "replace-local-secret"

mailboxes:
  - id: support
    address: support@example.com
    enabled: true
    poll_interval_seconds: 60
    safety_forward_to: ["human@example.com"]
    mcp_servers: ["dense_mem_primary"]
    agent:
      system_prompt_path: "support-agent.md"
      default_forward_to: ["human@example.com"]
    imap:
      host: imap.example.com
      port: 993
      tls: true
      username: support@example.com
      password: "replace-local-secret"
      folder: INBOX
      sent_folder: null
      sent_backfill_days: 30
    smtp:
      host: smtp.example.com
      port: 587
      starttls: true
      username: support@example.com
      password: "replace-local-secret"
      from: support@example.com
```

## Control Panel

The control panel is a React TypeScript dashboard served by the Rust web
service. It uses a `CONTROL_PANEL_KEY` login, same-origin API calls, SameSite
session cookies, and no CORS middleware.

Implemented foundation views:

- Overview
- History
- Rules
- Mailboxes
- MCP Servers
- Safety
- Settings

## Logging

The runtime uses structured action logging with `debug`, `info`, `warn`,
`error`, and `fatal` event levels. Rust's `tracing` ecosystem has standard
levels through `error`; `fatal` is represented as an application event level so
it can be stored and filtered consistently.

Every action log includes at least:

- `run_id`
- `mailbox_id`
- `message_uid` when available
- `action`
- `status`
- `duration_ms`
- `level`

Logs are emitted to stdout and persisted as PostgreSQL action events. The
default retention period is 180 days.

## Testing Policy

v1 requires the following local gates:

- backend unit test coverage of at least 90% for the unit-testable library
  surface
- frontend unit test coverage of at least 90%
- Playwright E2E tests for the control panel

Run all deterministic unit gates:

```bash
scripts/check-unit.sh
```

Run individual gates:

```bash
scripts/check-backend.sh
scripts/check-frontend.sh
cd web && npm run e2e
```

Run the opt-in live mail E2E directly:

```bash
AI_MEMMAIL_LIVE_E2E=1 AI_MEMMAIL_CONFIG=config/config.yaml \
  cargo test -p ai-memmail-server --test live_e2e -- --nocapture
```

The backend coverage gate excludes the binary entrypoint, raw external adapter
modules (`ai_external.rs`, `mail_external.rs`), and the Postgres-backed storage
adapter (`storage_pg.rs`) from the percentage calculation. Unit tests cover the
trait boundaries, parsing, payload mapping, fallbacks, and worker decisions.
Postgres storage tests are opt-in integration tests:

```bash
AI_MEMMAIL_RUN_POSTGRES_TESTS=1 \
AI_MEMMAIL_TEST_PG_HOST=127.0.0.1 \
AI_MEMMAIL_TEST_PG_PORT=5432 \
AI_MEMMAIL_TEST_PG_USER=postgres \
AI_MEMMAIL_TEST_PG_PASSWORD=postgres \
AI_MEMMAIL_TEST_PG_DATABASE=postgres \
  cargo test -p ai-memmail-server storage_pg -- --nocapture
```

Live mail, AI, and MCP behavior is covered by the opt-in live E2E because it
depends on local credentials and external services. GitHub CI runs deterministic
unit checks only. Live AI, Postgres storage integration, and E2E tests are local
only and use untracked credentials from `config/config.yaml`.

## Docker

The local Compose stack pulls `ghcr.io/markhuangai/ai-memmail:latest` by
default and runs:

- PostgreSQL
- `app`

The published image is a multi-stage build: Node builds the React control panel,
Rust builds the service binary, and the runtime image contains only the compiled
assets needed to run ai-memmail.

`scripts/live-e2e.sh` builds a local `AI_MEMMAIL_APP_IMAGE` tag before starting
the app container so the container E2E path tests the current checkout instead
of the published image.

## Git Vibe

This repository uses Git Vibe for planning, validation, and review automation.
Project-specific role prompts live in `.git-vibe/role-group/`, and the main
configuration is `.github/git-vibe.yml`.
