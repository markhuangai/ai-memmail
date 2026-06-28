import { FormEvent, useEffect, useMemo, useState } from "react";
import {
  Activity,
  KeyRound,
  LogOut,
  Mail,
  Plus,
  RefreshCw,
  Save,
  Server,
  Settings,
  ShieldAlert,
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
import { loadConfig, loadStatus, login, logout, saveConfig } from "./api";
import type { AppConfig, BannedSenderConfig, MailboxConfig, StatusResponse } from "./types";

type TabId = "overview" | "mailboxes" | "mcp" | "safety" | "settings";

const tabs: Array<{ id: TabId; label: string; icon: typeof Activity }> = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "mailboxes", label: "Mailboxes", icon: Mail },
  { id: "mcp", label: "MCP Servers", icon: Server },
  { id: "safety", label: "Safety", icon: ShieldAlert },
  { id: "settings", label: "Settings", icon: Settings }
];

export function App() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
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

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "request failed";
}
