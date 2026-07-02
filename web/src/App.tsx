import { FormEvent, useEffect, useMemo, useState } from "react";
import {
  Activity,
  History as HistoryIcon,
  KeyRound,
  LogOut,
  Mail,
  MessageSquareText,
  Plus,
  RefreshCw,
  Save,
  Server,
  Settings,
  ShieldAlert,
  Send,
  Trash2
} from "lucide-react";
import {
  addMailbox,
  addBannedSender,
  addMcpServer,
  displaySecret,
  envToText,
  listToText,
  mailboxRouteLabel,
  removeBannedSender,
  removeMailbox,
  removeMcpServer,
  setListFromText,
  textToEnv,
  summarizeConfig,
  updateMailbox,
  updateMcpServer
} from "./configModel";
import { loadConfig, loadMessages, loadStatus, login, logout, saveConfig } from "./api";
import type {
  AppConfig,
  BannedSenderConfig,
  MailboxConfig,
  ProcessedEmail,
  StatusResponse
} from "./types";

type TabId = "overview" | "history" | "mailboxes" | "mcp" | "safety" | "settings";

const tabs: Array<{ id: TabId; label: string; icon: typeof Activity }> = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "history", label: "History", icon: HistoryIcon },
  { id: "mailboxes", label: "Mailboxes", icon: Mail },
  { id: "mcp", label: "MCP Servers", icon: Server },
  { id: "safety", label: "Safety", icon: ShieldAlert },
  { id: "settings", label: "Settings", icon: Settings }
];

export function App() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [messages, setMessages] = useState<ProcessedEmail[]>([]);
  const [activeTab, setActiveTab] = useState<TabId>("overview");
  const [loginKey, setLoginKey] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  async function refresh() {
    setError("");
    const nextStatus = await loadStatus();
    setStatus(nextStatus);
    if (nextStatus.authenticated) {
      setConfig(await loadConfig());
      try {
        setMessages(await loadMessages());
      } catch (cause) {
        setMessages([]);
        setError(errorMessage(cause));
      }
    } else {
      setConfig(null);
      setMessages([]);
    }
  }

  useEffect(() => {
    refresh().catch((cause) => setError(errorMessage(cause)));
  }, []);

  async function onLogin(event: FormEvent) {
    event.preventDefault();
    setError("");
    await login(loginKey);
    setLoginKey("");
    await refresh();
  }

  async function onLogout() {
    setError("");
    await logout();
    setConfig(null);
    setMessages([]);
    await refresh();
  }

  async function onSave() {
    if (!config) {
      return;
    }
    setSaving(true);
    setError("");
    try {
      setConfig(await saveConfig(config));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  }

  const summary = useMemo(
    () => (config ? summarizeConfig(config) : null),
    [config]
  );

  if (!status?.authenticated) {
    return (
      <main className="login-shell">
        <form className="login-panel" onSubmit={onLogin}>
          <KeyRound aria-hidden="true" />
          <h1>ai-memmail</h1>
          <label>
            Control panel key
            <input
              autoFocus
              type="password"
              value={loginKey}
              onChange={(event) => setLoginKey(event.target.value)}
            />
          </label>
          <button type="submit">
            <KeyRound aria-hidden="true" />
            Login
          </button>
          {error ? <p role="alert">{error}</p> : null}
        </form>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <Mail aria-hidden="true" />
          <span>ai-memmail</span>
        </div>
        <nav aria-label="Primary">
          {tabs.map((tab) => {
            const Icon = tab.icon;
            return (
              <button
                className={activeTab === tab.id ? "active" : ""}
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                type="button"
              >
                <Icon aria-hidden="true" />
                {tab.label}
              </button>
            );
          })}
        </nav>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div>
            <h1>{tabs.find((tab) => tab.id === activeTab)?.label}</h1>
            <p>{status.enabled_mailboxes} enabled mailboxes</p>
          </div>
          <div className="topbar-actions">
            <button type="button" title="Refresh" onClick={() => refresh()}>
              <RefreshCw aria-hidden="true" />
              Refresh
            </button>
            <button type="button" title="Save" onClick={onSave} disabled={!config || saving}>
              <Save aria-hidden="true" />
              {saving ? "Saving" : "Save"}
            </button>
            <button type="button" title="Logout" onClick={onLogout}>
              <LogOut aria-hidden="true" />
              Logout
            </button>
          </div>
        </header>

        {error ? <div className="banner" role="alert">{error}</div> : null}
        {!config || !summary ? (
          <section className="panel">Loading</section>
        ) : (
          <section className="content-band">
            {activeTab === "overview" ? <Overview summary={summary} config={config} /> : null}
            {activeTab === "history" ? <HistoryPanel messages={messages} /> : null}
            {activeTab === "mailboxes" ? (
              <Mailboxes config={config} setConfig={setConfig} />
            ) : null}
            {activeTab === "mcp" ? (
              <McpServers config={config} setConfig={setConfig} />
            ) : null}
            {activeTab === "safety" ? (
              <Safety config={config} setConfig={setConfig} />
            ) : null}
            {activeTab === "settings" ? (
              <SettingsPanel config={config} setConfig={setConfig} />
            ) : null}
          </section>
        )}
      </main>
    </div>
  );
}

function HistoryPanel({ messages }: { messages: ProcessedEmail[] }) {
  const [selectedKey, setSelectedKey] = useState("");
  const selected = useMemo(() => {
    if (messages.length === 0) {
      return null;
    }
    return messages.find((message) => messageKey(message) === selectedKey) ?? messages[0];
  }, [messages, selectedKey]);
  const threadMessages = useMemo(() => {
    if (!selected) {
      return [];
    }
    return messages
      .filter((message) => message.thread_id === selected.thread_id)
      .sort((a, b) => timestampMs(a.created_at) - timestampMs(b.created_at));
  }, [messages, selected]);

  useEffect(() => {
    if (messages.length === 0) {
      setSelectedKey("");
      return;
    }
    if (!messages.some((message) => messageKey(message) === selectedKey)) {
      setSelectedKey(messageKey(messages[0]));
    }
  }, [messages, selectedKey]);

  if (messages.length === 0 || !selected) {
    return (
      <section className="panel">
        <h2>No processed messages</h2>
      </section>
    );
  }

  return (
    <div className="history-layout">
      <section className="panel message-list-panel">
        <div className="panel-heading">
          <div>
            <h2>Processed Email</h2>
            <p>{messages.length} messages</p>
          </div>
        </div>
        <div className="message-list" role="list">
          {messages.map((message) => {
            const key = messageKey(message);
            return (
              <button
                className={selected && messageKey(selected) === key ? "message-row active" : "message-row"}
                key={key}
                onClick={() => setSelectedKey(key)}
                type="button"
              >
                <span className="message-row-main">
                  <strong>{message.subject || "(no subject)"}</strong>
                  <span>{message.from_addr}</span>
                </span>
                <span className={statusPillClass(message.status)}>{message.status}</span>
                <span className="message-row-time">{formatTimestamp(message.updated_at)}</span>
              </button>
            );
          })}
        </div>
      </section>
      <MessageDetail
        message={selected}
        onSelectMessage={(message) => setSelectedKey(messageKey(message))}
        threadMessages={threadMessages}
      />
    </div>
  );
}

function MessageDetail({
  message,
  onSelectMessage,
  threadMessages
}: {
  message: ProcessedEmail;
  onSelectMessage: (message: ProcessedEmail) => void;
  threadMessages: ProcessedEmail[];
}) {
  return (
    <section className="panel message-detail-panel">
      <div className="panel-heading">
        <div>
          <h2>{message.subject || "(no subject)"}</h2>
          <p>{message.from_addr}</p>
        </div>
        <span className={statusPillClass(message.status)}>{message.status}</span>
      </div>

      <dl className="detail-grid message-detail-grid">
        <div>
          <dt>Mailbox</dt>
          <dd>{message.mailbox_id}</dd>
        </div>
        <div>
          <dt>UID</dt>
          <dd>{message.uid_validity}:{message.uid}</dd>
        </div>
        <div>
          <dt>Message ID</dt>
          <dd>{message.message_id ?? "none"}</dd>
        </div>
        <div>
          <dt>Thread</dt>
          <dd>{message.thread_id}</dd>
        </div>
        <div>
          <dt>Updated</dt>
          <dd>{formatTimestamp(message.updated_at)}</dd>
        </div>
      </dl>

      <section className="message-section">
        <h3><Mail aria-hidden="true" /> Inbound</h3>
        <dl className="detail-grid message-detail-grid">
          <div>
            <dt>In reply to</dt>
            <dd>{message.in_reply_to ?? "none"}</dd>
          </div>
          <div>
            <dt>References</dt>
            <dd>{message.references.length ? message.references.join(", ") : "none"}</dd>
          </div>
        </dl>
        {message.inbound_body ? (
          <>
            <pre className="message-body">{message.inbound_body}</pre>
            {message.inbound_body_truncated ? (
              <p className="muted">Inbound body truncated for storage.</p>
            ) : null}
          </>
        ) : (
          <p className="muted">No inbound body recorded.</p>
        )}
      </section>

      <section className="message-section">
        <h3><MessageSquareText aria-hidden="true" /> Email chain</h3>
        <div className="chain-list" role="list">
          {threadMessages.map((threadMessage) => {
            const active = messageKey(threadMessage) === messageKey(message);
            return (
              <button
                className={active ? "chain-item active" : "chain-item"}
                key={messageKey(threadMessage)}
                onClick={() => onSelectMessage(threadMessage)}
                type="button"
              >
                <span>
                  <strong>{threadMessage.subject || "(no subject)"}</strong>
                  <span>{threadMessage.from_addr}</span>
                </span>
                <span className={statusPillClass(threadMessage.status)}>{threadMessage.status}</span>
                <span>{formatTimestamp(threadMessage.updated_at)}</span>
              </button>
            );
          })}
        </div>
      </section>

      <section className="message-section">
        <h3><ShieldAlert aria-hidden="true" /> Safety and AI</h3>
        <dl className="detail-grid message-detail-grid">
          <div>
            <dt>Safety</dt>
            <dd>{message.safety_category ?? "not recorded"}</dd>
          </div>
          <div>
            <dt>Agent action</dt>
            <dd>{message.agent_action ?? "not recorded"}</dd>
          </div>
          <div>
            <dt>Review</dt>
            <dd>{message.outbound_review_status ?? "not reviewed"}</dd>
          </div>
          <div>
            <dt>Final action</dt>
            <dd>{message.outbound_action ?? "not recorded"}</dd>
          </div>
        </dl>
        <TextBlock label="Safety reason" value={message.safety_reason} />
        <TextBlock label="Agent notes" value={message.agent_safety_notes} />
        <TextBlock label="Review reason" value={message.outbound_review_reason} />
      </section>

      <section className="message-section">
        <h3><Send aria-hidden="true" /> Outbound</h3>
        <dl className="detail-grid message-detail-grid">
          <div>
            <dt>Recipients</dt>
            <dd>{message.outbound_recipients.length ? message.outbound_recipients.join(", ") : "none"}</dd>
          </div>
          <div>
            <dt>Subject</dt>
            <dd>{message.outbound_subject ?? "none"}</dd>
          </div>
          <div>
            <dt>Reason</dt>
            <dd>{message.outbound_reason ?? "none"}</dd>
          </div>
          <div>
            <dt>Outbound Message ID</dt>
            <dd>{message.outbound_message_id ?? "none"}</dd>
          </div>
        </dl>
        {message.outbound_body ? (
          <pre className="message-body">{message.outbound_body}</pre>
        ) : message.outbound_body_redacted ? (
          <p className="muted">Forward body omitted because it can include original inbound email content.</p>
        ) : (
          <p className="muted">No outbound body recorded.</p>
        )}
      </section>

      <section className="message-section">
        <h3><MessageSquareText aria-hidden="true" /> Timeline</h3>
        {message.logs.length === 0 ? (
          <p className="muted">No log entries recorded.</p>
        ) : (
          <div className="table-wrap">
            <table className="timeline-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Level</th>
                  <th>Action</th>
                  <th>Status</th>
                  <th>Detail</th>
                </tr>
              </thead>
              <tbody>
                {message.logs.map((entry, index) => (
                  <tr key={`${entry.created_at}:${entry.action}:${index}`}>
                    <td>{formatTimestamp(entry.created_at)}</td>
                    <td>{entry.level}</td>
                    <td>{entry.action}</td>
                    <td>{entry.status}</td>
                    <td>{entry.detail ?? ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </section>
  );
}

function TextBlock({ label, value }: { label: string; value?: string | null }) {
  if (!value) {
    return null;
  }
  return (
    <div className="text-block">
      <span>{label}</span>
      <p>{value}</p>
    </div>
  );
}

function Overview({ summary, config }: { summary: ReturnType<typeof summarizeConfig>; config: AppConfig }) {
  return (
    <div className="overview-grid">
      <Metric label="Mailboxes" value={`${summary.enabledMailboxes}/${summary.mailboxCount}`} />
      <Metric label="MCP servers" value={String(summary.mcpServerCount)} />
      <Metric label="Avg poll" value={`${summary.averagePollSeconds}s`} />
      <Metric label="Banned senders" value={String(summary.bannedSenderCount)} />
      <section className="panel wide">
        <h2>Runtime</h2>
        <dl className="detail-grid">
          <div>
            <dt>AI model</dt>
            <dd>{config.ai.AI_MODEL}</dd>
          </div>
          <div>
            <dt>AI secret</dt>
            <dd>{displaySecret(config.ai.AI_API_SECRET)}</dd>
          </div>
          <div>
            <dt>Safety prompt</dt>
            <dd>{config.prompts.safety_scan}</dd>
          </div>
          <div>
            <dt>Postgres</dt>
            <dd>{config.database.host}:{config.database.port}</dd>
          </div>
          <div>
            <dt>Log retention</dt>
            <dd>{config.logging.retention_days} days</dd>
          </div>
        </dl>
      </section>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <section className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </section>
  );
}

function Mailboxes({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  function patchMailbox(mailboxId: string, updater: (mailbox: MailboxConfig) => MailboxConfig) {
    setConfig(updateMailbox(config, mailboxId, updater));
  }

  return (
    <div className="stack">
      <div className="section-actions">
        <button type="button" onClick={() => setConfig(addMailbox(config))}>
          <Plus aria-hidden="true" />
          Add mailbox
        </button>
      </div>
      {config.mailboxes.length === 0 ? (
        <section className="panel">
          <h2>No mailboxes configured</h2>
        </section>
      ) : null}
      {config.mailboxes.map((mailbox) => (
        <section className="panel" key={mailbox.id}>
          <div className="panel-heading">
            <div>
              <h2>{mailbox.address}</h2>
              <p>{mailboxRouteLabel(mailbox)}</p>
            </div>
            <div className="panel-actions">
              <label className="switch">
                <input
                  type="checkbox"
                  checked={mailbox.enabled}
                  onChange={(event) =>
                    patchMailbox(mailbox.id, (next) => ({
                      ...next,
                      enabled: event.target.checked
                    }))
                  }
                />
                Enabled
              </label>
              <button type="button" onClick={() => setConfig(removeMailbox(config, mailbox.id))}>
                <Trash2 aria-hidden="true" />
                Remove
              </button>
            </div>
          </div>

          <div className="form-grid">
            <label>
              Address
              <input
                value={mailbox.address}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    address: event.target.value
                  }))
                }
              />
            </label>
            <label>
              Poll seconds
              <input
                type="number"
                min="1"
                value={mailbox.poll_interval_seconds}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    poll_interval_seconds: Number(event.target.value)
                  }))
                }
              />
            </label>
            <label>
              Safety forward
              <input
                value={listToText(mailbox.safety_forward_to)}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    safety_forward_to: setListFromText(event.target.value)
                  }))
                }
              />
            </label>
            <label>
              MCP servers
              <input
                value={listToText(mailbox.mcp_servers)}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    mcp_servers: setListFromText(event.target.value)
                  }))
                }
              />
            </label>
            <label>
              IMAP host
              <input
                value={mailbox.imap.host}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    imap: { ...next.imap, host: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              IMAP port
              <input
                type="number"
                min="1"
                value={mailbox.imap.port}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    imap: { ...next.imap, port: Number(event.target.value) }
                  }))
                }
              />
            </label>
            <label>
              IMAP user
              <input
                value={mailbox.imap.username}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    imap: { ...next.imap, username: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              IMAP password
              <input
                type="password"
                value={mailbox.imap.password}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    imap: { ...next.imap, password: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              IMAP folder
              <input
                value={mailbox.imap.folder}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    imap: { ...next.imap, folder: event.target.value }
                  }))
                }
              />
            </label>
            <label className="switch">
              <input
                type="checkbox"
                checked={mailbox.imap.tls}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    imap: { ...next.imap, tls: event.target.checked }
                  }))
                }
              />
              IMAP TLS
            </label>
            <label>
              SMTP host
              <input
                value={mailbox.smtp.host}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    smtp: { ...next.smtp, host: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              SMTP port
              <input
                type="number"
                min="1"
                value={mailbox.smtp.port}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    smtp: { ...next.smtp, port: Number(event.target.value) }
                  }))
                }
              />
            </label>
            <label>
              SMTP user
              <input
                value={mailbox.smtp.username}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    smtp: { ...next.smtp, username: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              SMTP password
              <input
                type="password"
                value={mailbox.smtp.password}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    smtp: { ...next.smtp, password: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              SMTP from
              <input
                value={mailbox.smtp.from}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    smtp: { ...next.smtp, from: event.target.value }
                  }))
                }
              />
            </label>
            <label className="switch">
              <input
                type="checkbox"
                checked={mailbox.smtp.starttls}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    smtp: { ...next.smtp, starttls: event.target.checked }
                  }))
                }
              />
              SMTP STARTTLS
            </label>
            <label>
              Agent prompt
              <input
                value={mailbox.agent.system_prompt_path}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    agent: { ...next.agent, system_prompt_path: event.target.value }
                  }))
                }
              />
            </label>
            <label>
              Default forward
              <input
                value={listToText(mailbox.agent.default_forward_to)}
                onChange={(event) =>
                  patchMailbox(mailbox.id, (next) => ({
                    ...next,
                    agent: {
                      ...next.agent,
                      default_forward_to: setListFromText(event.target.value)
                    }
                  }))
                }
              />
            </label>
          </div>
        </section>
      ))}
    </div>
  );
}

function McpServers({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const servers = Object.entries(config.mcp_servers);
  return (
    <div className="stack">
      <div className="section-actions">
        <button type="button" onClick={() => setConfig(addMcpServer(config))}>
          <Plus aria-hidden="true" />
          Add server
        </button>
      </div>
      {servers.length === 0 ? (
        <section className="panel">
          <h2>No MCP servers configured</h2>
        </section>
      ) : null}
      {servers.map(([name, server]) => (
        <section className="panel" key={name}>
          <div className="panel-heading">
            <div>
              <h2>{name}</h2>
              <p>{server.transport}</p>
            </div>
            <button type="button" onClick={() => setConfig(removeMcpServer(config, name))}>
              <Trash2 aria-hidden="true" />
              Remove
            </button>
          </div>
          <div className="form-grid">
            <label>
              Server id
              <input value={name} readOnly />
            </label>
            <label>
              Transport
              <select
                value={server.transport}
                onChange={(event) =>
                  setConfig(
                    updateMcpServer(config, name, (next) => ({
                      ...next,
                      transport: event.target.value as AppConfig["mcp_servers"][string]["transport"]
                    }))
                  )
                }
              >
                <option value="stdio">stdio</option>
                <option value="streamable_http">streamable_http</option>
              </select>
            </label>
            <label>
              Command
              <input
                value={server.command ?? ""}
                onChange={(event) =>
                  setConfig(
                    updateMcpServer(config, name, (next) => ({
                      ...next,
                      command: event.target.value || null
                    }))
                  )
                }
              />
            </label>
            <label>
              URL
              <input
                value={server.url ?? ""}
                onChange={(event) =>
                  setConfig(
                    updateMcpServer(config, name, (next) => ({
                      ...next,
                      url: event.target.value || null
                    }))
                  )
                }
              />
            </label>
            <label>
              Args
              <input
                value={listToText(server.args)}
                onChange={(event) =>
                  setConfig(
                    updateMcpServer(config, name, (next) => ({
                      ...next,
                      args: setListFromText(event.target.value)
                    }))
                  )
                }
              />
            </label>
            <label>
              Env
              <textarea
                value={envToText(server.env)}
                onChange={(event) =>
                  setConfig(
                    updateMcpServer(config, name, (next) => ({
                      ...next,
                      env: textToEnv(event.target.value)
                    }))
                  )
                }
              />
            </label>
          </div>
        </section>
      ))}
    </div>
  );
}

function Safety({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const [draft, setDraft] = useState<BannedSenderConfig>({
    kind: "email",
    value: "",
    reason: ""
  });

  function addDraft() {
    if (!draft.value.trim() || !draft.reason.trim()) {
      return;
    }
    setConfig(addBannedSender(config, { ...draft, value: draft.value.trim() }));
    setDraft({ kind: "email", value: "", reason: "" });
  }

  return (
    <div className="stack">
      <section className="panel">
        <h2>Banned Senders</h2>
        <div className="inline-form">
          <select
            aria-label="Ban kind"
            value={draft.kind}
            onChange={(event) =>
              setDraft({ ...draft, kind: event.target.value as BannedSenderConfig["kind"] })
            }
          >
            <option value="email">email</option>
            <option value="domain">domain</option>
          </select>
          <input
            aria-label="Ban value"
            value={draft.value}
            onChange={(event) => setDraft({ ...draft, value: event.target.value })}
          />
          <input
            aria-label="Ban reason"
            value={draft.reason}
            onChange={(event) => setDraft({ ...draft, reason: event.target.value })}
          />
          <button type="button" onClick={addDraft}>Add</button>
        </div>
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Kind</th>
                <th>Value</th>
                <th>Reason</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {config.banned_senders.map((sender) => (
                <tr key={`${sender.kind}:${sender.value}`}>
                  <td>{sender.kind}</td>
                  <td>{sender.value}</td>
                  <td>{sender.reason}</td>
                  <td>
                    <button
                      type="button"
                      onClick={() => setConfig(removeBannedSender(config, sender))}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

function SettingsPanel({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  return (
    <section className="panel">
      <div className="form-grid">
        <label>
          Postgres host
          <input
            value={config.database.host}
            onChange={(event) =>
              setConfig({
                ...config,
                database: { ...config.database, host: event.target.value }
              })
            }
          />
        </label>
        <label>
          Postgres port
          <input
            type="number"
            min="1"
            value={config.database.port}
            onChange={(event) =>
              setConfig({
                ...config,
                database: { ...config.database, port: Number(event.target.value) }
              })
            }
          />
        </label>
        <label>
          Postgres user
          <input
            value={config.database.username}
            onChange={(event) =>
              setConfig({
                ...config,
                database: { ...config.database, username: event.target.value }
              })
            }
          />
        </label>
        <label>
          Postgres password
          <input
            type="password"
            value={config.database.password}
            onChange={(event) =>
              setConfig({
                ...config,
                database: { ...config.database, password: event.target.value }
              })
            }
          />
        </label>
        <label>
          Postgres database
          <input
            value={config.database.database}
            onChange={(event) =>
              setConfig({
                ...config,
                database: { ...config.database, database: event.target.value }
              })
            }
          />
        </label>
        <label>
          AI API URL
          <input
            value={config.ai.AI_API_URL}
            onChange={(event) =>
              setConfig({
                ...config,
                ai: { ...config.ai, AI_API_URL: event.target.value }
              })
            }
          />
        </label>
        <label>
          AI model
          <input
            value={config.ai.AI_MODEL}
            onChange={(event) =>
              setConfig({
                ...config,
                ai: { ...config.ai, AI_MODEL: event.target.value }
              })
            }
          />
        </label>
        <label>
          Prompt root
          <input
            value={config.prompts.root}
            onChange={(event) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, root: event.target.value }
              })
            }
          />
        </label>
        <label>
          Safety prompt
          <input
            value={config.prompts.safety_scan}
            onChange={(event) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, safety_scan: event.target.value }
              })
            }
          />
        </label>
        <label>
          Log level
          <select
            value={config.logging.level}
            onChange={(event) =>
              setConfig({
                ...config,
                logging: {
                  ...config.logging,
                  level: event.target.value as AppConfig["logging"]["level"]
                }
              })
            }
          >
            <option value="debug">debug</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>
        </label>
        <label>
          Retention days
          <input
            type="number"
            min="1"
            value={config.logging.retention_days}
            onChange={(event) =>
              setConfig({
                ...config,
                logging: {
                  ...config.logging,
                  retention_days: Number(event.target.value)
                }
              })
            }
          />
        </label>
      </div>
    </section>
  );
}

function messageKey(message: ProcessedEmail): string {
  return `${message.mailbox_id}:${message.uid_validity}:${message.uid}`;
}

function statusPillClass(status: string): string {
  if (status.includes("failed")) {
    return "status-pill danger";
  }
  if (status === "processing") {
    return "status-pill pending";
  }
  if (["replied", "forwarded", "noop", "quarantined"].includes(status)) {
    return "status-pill success";
  }
  return "status-pill";
}

function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString();
}

function timestampMs(value: string): number {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? 0 : parsed.getTime();
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "request failed";
}
