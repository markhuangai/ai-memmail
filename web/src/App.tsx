import { FormEvent, useEffect, useMemo, useState } from "react";
import {
  Activity,
  FileText,
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
  Tags,
  Trash2
} from "lucide-react";
import { summarizeConfig } from "./configModel";
import { createHandoff, loadConfig, loadEmailClassification, loadMessages, loadStatus, login, logout, saveConfig } from "./api";
import { HistoryPanel } from "./panels/HistoryPanel";
import { Mailboxes } from "./panels/MailboxesPanel";
import { McpServers } from "./panels/McpServersPanel";
import { Overview } from "./panels/OverviewPanel";
import { RulesPanel } from "./panels/RulesPanel";
import { Safety } from "./panels/SafetyPanel";
import { SettingsPanel } from "./panels/SettingsPanel";
import type { AppConfig, EmailClassificationConfig, ProcessedEmail, StatusResponse } from "./types";
import { errorMessage } from "./viewUtils";

type TabId = "overview" | "history" | "rules" | "mailboxes" | "mcp" | "safety" | "settings";

const DEFAULT_HISTORY_LIMIT = 100;
const HISTORY_LIMIT_STEP = 100;
const MAX_HISTORY_LIMIT = 500;

const tabs: Array<{ id: TabId; label: string; icon: typeof Activity }> = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "history", label: "History", icon: HistoryIcon },
  { id: "rules", label: "Rules", icon: Tags },
  { id: "mailboxes", label: "Mailboxes", icon: Mail },
  { id: "mcp", label: "MCP Servers", icon: Server },
  { id: "safety", label: "Safety", icon: ShieldAlert },
  { id: "settings", label: "Settings", icon: Settings }
];

export function App({
  initialHistoryLimit = DEFAULT_HISTORY_LIMIT
}: {
  initialHistoryLimit?: number;
} = {}) {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [messages, setMessages] = useState<ProcessedEmail[]>([]);
  const [classification, setClassification] = useState<EmailClassificationConfig | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("overview");
  const [loginKey, setLoginKey] = useState("");
  const [messageLimit, setMessageLimit] = useState(initialHistoryLimit);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  async function refresh(nextMessageLimit = messageLimit) {
    setError("");
    const nextStatus = await loadStatus();
    setStatus(nextStatus);
    if (nextStatus.authenticated) {
      setConfig(await loadConfig());
      try {
        setMessages(await loadMessages(nextMessageLimit));
      } catch (cause) {
        setMessages([]);
        setError(errorMessage(cause));
      }
      try {
        setClassification(await loadEmailClassification());
      } catch (cause) {
        setClassification(null);
        setError(errorMessage(cause));
      }
    } else {
      setConfig(null);
      setMessages([]);
      setClassification(null);
    }
  }

  async function loadMoreHistory() {
    const nextMessageLimit = Math.min(messageLimit + HISTORY_LIMIT_STEP, MAX_HISTORY_LIMIT);
    setError("");
    try {
      setMessages(await loadMessages(nextMessageLimit));
      setMessageLimit(nextMessageLimit);
    } catch (cause) {
      setError(errorMessage(cause));
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
    setClassification(null);
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

  async function onCreateHandoff(message: ProcessedEmail, destination: string) {
    setError("");
    try {
      await createHandoff(message.run_id, destination);
      setMessages(await loadMessages(messageLimit));
    } catch (cause) {
      setError(errorMessage(cause));
      throw cause;
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
            {activeTab === "history" ? (
              <HistoryPanel
                canLoadMore={messages.length >= messageLimit && messageLimit < MAX_HISTORY_LIMIT}
                messageLimit={messageLimit}
                messages={messages}
                onCreateHandoff={onCreateHandoff}
                onLoadMore={loadMoreHistory}
              />
            ) : null}
            {activeTab === "rules" ? (
              <RulesPanel
                classification={classification}
                config={config}
                setClassification={setClassification}
                setError={setError}
              />
            ) : null}
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
