# ai-memmail

ai-memmail is a Rust email agent that monitors IMAP mailboxes, uses configured
Dense-Mem MCP servers to answer mail when policy allows it, and sends replies or
forwards through SMTP. The control panel is a same-origin React TypeScript
dashboard served by the Rust web service.

## Current Status

The repository now contains the v1 application foundation:

- Rust workspace with an Axum control-panel API, IMAP polling worker,
  OpenAI-compatible safety/agent decisions, Dense-Mem MCP recall over HTTP,
  SMTP reply/forward sending, typed YAML configuration, prompt-file loading,
  safety policy primitives, structured action logging, and PostgreSQL
  migrations.
- React TypeScript control panel for login, mailbox settings, MCP server
  settings, safety lists, AI prompt paths, and logging settings.
- Source-built Docker runtime that runs the web API and worker side by side by
  default, with explicit `web` and `worker` roles still available.
- Backend and frontend unit coverage gates, plus Playwright E2E coverage for the
  control panel.

PostgreSQL persistence tracks processing run state, processed-message history,
structured decisions, outbound actions, and action logs. The worker still relies
on IMAP `UNSEEN` plus `Seen` marking for source-mail delivery state.

## Local Setup

Create a local config and panel key:

```bash
cp config/config.example.yaml config/config.yaml
export CONTROL_PANEL_KEY="replace-with-local-key"
```

Run the full local stack from source:

```bash
docker compose up --build
```

The control panel is served at `http://127.0.0.1:18080` by default. PostgreSQL
is exposed on `127.0.0.1:15432` by default to avoid conflicts with a host
Postgres install. To change either host port, edit the port forwarding in
`docker-compose.yml`.

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

The live mail test loads `config/config.yaml`, sends one email scenario at a time,
runs the real worker path, waits for the expected reply or forward, then moves
to the next scenario. It covers MCP-backed known-answer reply, explicit
human-forward routing, prompt-injection quarantine forwarding, and banned-sender
forwarding. The test derives the forward mailbox credentials from the local
config in memory; do not commit local credentials.

## v1 Architecture

```text
IMAP inbox
  -> worker fetches message metadata and body
  -> metadata is stored in PostgreSQL
  -> sender ban/review checks run
  -> safety scan runs with no MCP tools and no send capability
      -> flagged: quarantine, add sender to review, forward to human reviewer
      -> safe: run mailbox agent with allowed MCP tools
  -> SMTP reply or forward
  -> action logs go to stdout and PostgreSQL
  -> control panel shows status, safety queue, sender review, and logs
```

The production runtime uses one source-built image. By default it starts both
long-running paths in one process:

- `web`: Axum API and React control panel.
- `worker`: IMAP polling, safety scanning, AI/MCP processing, and SMTP sending.

Set `AI_MEMMAIL_ROLE=web` or `AI_MEMMAIL_ROLE=worker` to run only one path.

At process startup, `ai-memmail-server` connects to PostgreSQL and applies
versioned SQL migrations before starting the selected role. Migration application
uses a PostgreSQL advisory lock and records applied versions plus checksums in
`schema_migrations`, so concurrent replicas serialize schema changes and detect
edited historical migrations.

Both roles share PostgreSQL. Raw inbound email bodies are not stored in
PostgreSQL; metadata, processing decisions, safety results, outbound reply
bodies, sender review state, banned senders, and action logs are persisted.
Forward bodies are redacted before storage because they can include original
inbound content. Reprocessing refetches source messages from IMAP.

## Mail Flow

v1 uses password-authenticated IMAP/IMAPS for inbound mail and SMTP/SMTPS for
outbound mail. OAuth2 is intentionally out of scope for v1.

After successful processing, ai-memmail marks the source message as `Seen`.
Deduplication uses mailbox id plus IMAP `UIDVALIDITY` and UID so repeated polls
do not send duplicate replies.

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
7. If safe, run the mailbox agent with only the mailbox's allowed MCP servers.
8. Optionally run a second outbound AI review pass. It exists in v1 but is
   disabled by default.
9. Send only if deterministic validation accepts the structured AI result.

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

The backend coverage gate excludes the binary entrypoint and raw external
adapter modules (`ai_external.rs`, `mail_external.rs`) from the percentage
calculation. Unit tests cover the trait boundaries, parsing, payload mapping,
fallbacks, and worker decisions; live mail, AI, and MCP behavior is covered by
the opt-in live E2E because it depends on local credentials and external
services.
GitHub CI runs deterministic unit checks only. Live AI and E2E tests are local
only and use untracked credentials from `config/config.yaml`.

## Docker

The local Compose stack builds from source and runs:

- PostgreSQL
- `app`

The image is a multi-stage build: Node builds the React control panel, Rust
builds the service binary, and the runtime image contains only the compiled
assets needed to run ai-memmail.

## Git Vibe

This repository uses Git Vibe for planning, validation, and review automation.
Project-specific role prompts live in `.git-vibe/role-group/`, and the main
configuration is `.github/git-vibe.yml`.
