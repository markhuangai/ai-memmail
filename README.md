# ai-memmail

ai-memmail is a Rust email agent that monitors IMAP mailboxes, uses configured
Dense-Mem MCP servers to answer mail when policy allows it, and sends replies or
forwards through SMTP. The control panel is a same-origin React TypeScript
dashboard served by the Rust web service.

## Current Status

This repository is being initialized in two steps.

1. Bootstrap `main` with this README and Git Vibe review configuration.
2. Build the v1 application foundation on a feature branch from local `main`.

The bootstrap commit does not include the Rust service, React panel, Docker
runtime, migrations, or test suite yet.

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

The production runtime uses one source-built image with two roles:

- `web`: Axum API and React control panel.
- `worker`: IMAP polling, safety scanning, AI/MCP processing, and SMTP sending.

Both roles share PostgreSQL. Email content is not stored in PostgreSQL; only
metadata, processing decisions, safety results, sender review state, banned
senders, and action logs are persisted. Reprocessing refetches source messages
from IMAP.

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

- `.ai-cred`
- `.env.local`
- `config/local.yaml`
- `config/live.local.yaml`

System prompts are configured as file paths, not inline prompt text. Paths are
resolved relative to `prompts.root`.

Example shape:

```yaml
version: 1

database:
  url: "postgres://ai_memmail:ai_memmail@postgres:5432/ai_memmail"

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

Expected v1 views:

- Overview
- Mailboxes
- MCP Servers
- AI Policy
- Safety Queue
- Sender Review
- Banned Senders
- Activity Logs
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

v1 requires:

- backend unit test coverage of at least 90%
- frontend unit test coverage of at least 90%
- local Playwright E2E tests

GitHub CI runs deterministic unit checks only. Live AI and E2E tests are local
only and use untracked credentials from `.ai-cred` or `config/live.local.yaml`.

## Docker

The local Compose stack will build from source and run:

- PostgreSQL
- `web`
- `worker`

The production image will be a multi-stage build: Node builds the React control
panel, Rust builds the service binary, and the runtime image contains only the
compiled assets needed to run ai-memmail.

## Git Vibe

This repository uses Git Vibe for planning, validation, and review automation.
Project-specific role prompts live in `.git-vibe/role-group/`, and the main
configuration is `.github/git-vibe.yml`.
