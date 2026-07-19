import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  History as HistoryIcon,
  KeyRound,
  LogOut,
  Mail,
  Menu,
  RefreshCw,
  Save,
  Server,
  Settings,
  ShieldAlert,
  Tags,
  X
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
import { ConfirmDialog } from "./panels/ConfirmDialog";
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

const configTabs = new Set<TabId>(["mailboxes", "mcp", "safety", "settings"]);

type PendingGuardAction = "refresh" | "logout";
type PendingGuardRequest = {
  action: PendingGuardAction;
  id: number;
};

export function App({
  initialHistoryLimit = DEFAULT_HISTORY_LIMIT
}: {
  initialHistoryLimit?: number;
} = {}) {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [savedConfig, setSavedConfig] = useState<AppConfig | null>(null);
  const [draftConfig, setDraftConfig] = useState<AppConfig | null>(null);
  const [messages, setMessages] = useState<ProcessedEmail[]>([]);
  const [classification, setClassification] = useState<EmailClassificationConfig | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("overview");
  const [lastEditedConfigTab, setLastEditedConfigTab] = useState<TabId>("mailboxes");
  const [pendingGuardRequest, setPendingGuardRequestState] = useState<PendingGuardRequest | null>(null);
  const pendingGuardRequestRef = useRef<PendingGuardRequest | null>(null);
  const nextPendingGuardRequestId = useRef(0);
  const [navOpen, setNavOpen] = useState(false);
  const [loginKey, setLoginKey] = useState("");
  const [messageLimit, setMessageLimit] = useState(initialHistoryLimit);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  const isConfigDirty = useMemo(
    () => Boolean(savedConfig && draftConfig && !configsEqual(savedConfig, draftConfig)),
    [savedConfig, draftConfig]
  );
  const activeConfigTab = configTabs.has(activeTab);

  function setPendingGuardRequest(action: PendingGuardAction | null) {
    if (!action) {
      pendingGuardRequestRef.current = null;
      setPendingGuardRequestState(null);
      return;
    }
    const request = {
      action,
      id: nextPendingGuardRequestId.current + 1
    };
    nextPendingGuardRequestId.current = request.id;
    pendingGuardRequestRef.current = request;
    setPendingGuardRequestState(request);
  }

  async function refreshFromServer(nextMessageLimit = messageLimit) {
    setError("");
    const nextStatus = await loadStatus();
    setStatus(nextStatus);
    if (nextStatus.authenticated) {
      const nextConfig = await loadConfig();
      setSavedConfig(nextConfig);
      setDraftConfig(nextConfig);
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
      setSavedConfig(null);
      setDraftConfig(null);
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
    refreshFromServer().catch((cause) => setError(errorMessage(cause)));
  }, []);

  useEffect(() => {
    if (!isConfigDirty) {
      return;
    }
    function warnBeforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "";
    }
    window.addEventListener("beforeunload", warnBeforeUnload);
    return () => window.removeEventListener("beforeunload", warnBeforeUnload);
  }, [isConfigDirty]);

  async function onLogin(event: FormEvent) {
    event.preventDefault();
    setError("");
    await login(loginKey);
    setLoginKey("");
    await refreshFromServer();
  }

  async function logoutNow() {
    setError("");
    await logout();
    setSavedConfig(null);
    setDraftConfig(null);
    setMessages([]);
    setClassification(null);
    await refreshFromServer();
  }

  async function saveDraftConfig() {
    if (!draftConfig) {
      return false;
    }
    setSaving(true);
    setError("");
    try {
      const nextConfig = await saveConfig(draftConfig);
      setSavedConfig(nextConfig);
      setDraftConfig(nextConfig);
      return true;
    } catch (cause) {
      setError(errorMessage(cause));
      return false;
    } finally {
      setSaving(false);
    }
  }

  function applyDraftConfig(nextConfig: AppConfig) {
    setDraftConfig(nextConfig);
    if (activeConfigTab) {
      setLastEditedConfigTab(activeTab);
    }
  }

  function requestRefresh() {
    if (isConfigDirty) {
      setPendingGuardRequest("refresh");
      return;
    }
    refreshFromServer().catch((cause) => setError(errorMessage(cause)));
  }

  function requestLogout() {
    if (isConfigDirty) {
      setPendingGuardRequest("logout");
      return;
    }
    logoutNow().catch((cause) => setError(errorMessage(cause)));
  }

  async function continueAfterSaving() {
    const request = pendingGuardRequestRef.current;
    if (!request) {
      return;
    }
    const saved = await saveDraftConfig();
    if (!saved || pendingGuardRequestRef.current !== request) {
      return;
    }
    setPendingGuardRequest(null);
    if (request.action === "refresh") {
      await refreshFromServer();
      return;
    }
    await logoutNow();
  }

  function continueAfterDiscarding() {
    const request = pendingGuardRequestRef.current;
    if (!request) {
      return;
    }
    if (savedConfig) {
      setDraftConfig(savedConfig);
    }
    setPendingGuardRequest(null);
    if (request.action === "refresh") {
      refreshFromServer().catch((cause) => setError(errorMessage(cause)));
      return;
    }
    logoutNow().catch((cause) => setError(errorMessage(cause)));
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
    () => (draftConfig ? summarizeConfig(draftConfig) : null),
    [draftConfig]
  );
  const activeTabMeta = tabs.find((tab) => tab.id === activeTab) ?? tabs[0];
  const dirtyTabLabel = tabs.find((tab) => tab.id === lastEditedConfigTab)?.label ?? "Config";

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
      <aside className={navOpen ? "sidebar open" : "sidebar"}>
        <div className="brand">
          <Mail aria-hidden="true" />
          <span>ai-memmail</span>
          <button
            aria-label="Close navigation"
            className="mobile-nav-close"
            type="button"
            onClick={() => setNavOpen(false)}
          >
            <X aria-hidden="true" />
          </button>
        </div>
        <nav aria-label="Primary">
          {tabs.map((tab) => {
            const Icon = tab.icon;
            return (
              <button
                className={activeTab === tab.id ? "active" : ""}
                key={tab.id}
                title={tab.label}
                onClick={() => {
                  setActiveTab(tab.id);
                  setNavOpen(false);
                }}
                type="button"
              >
                <Icon aria-hidden="true" />
                {tab.label}
              </button>
            );
          })}
        </nav>
        <div className="sidebar-footer">
          {isConfigDirty ? (
            <button
              className="dirty-return"
              type="button"
              onClick={() => {
                setActiveTab(lastEditedConfigTab);
                setNavOpen(false);
              }}
            >
              <span>Draft changes</span>
              <small>{dirtyTabLabel}</small>
            </button>
          ) : (
            <p className="saved-state">Config saved</p>
          )}
          <button className="signout-action" type="button" onClick={requestLogout}>
            <LogOut aria-hidden="true" />
            Sign out
          </button>
        </div>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <button
            aria-label="Open navigation"
            className="mobile-nav-toggle"
            type="button"
            onClick={() => setNavOpen(true)}
          >
            <Menu aria-hidden="true" />
          </button>
          <div className="topbar-title">
            <h1>{activeTabMeta.label}</h1>
            <p>{topbarSubtitle(activeTab, status, messages, summary)}</p>
          </div>
          <div className="topbar-actions">
            <button
              aria-label="Refresh data"
              className="icon-action"
              title="Refresh data"
              type="button"
              onClick={requestRefresh}
            >
              <RefreshCw aria-hidden="true" />
            </button>
            {activeConfigTab ? (
              <button
                className="primary-action"
                type="button"
                title="Save changes"
                onClick={() => {
                  saveDraftConfig().catch((cause) => setError(errorMessage(cause)));
                }}
                disabled={!draftConfig || saving || !isConfigDirty}
              >
                <Save aria-hidden="true" />
                {saving ? "Saving" : "Save changes"}
              </button>
            ) : null}
          </div>
        </header>

        {error ? <div className="banner" role="alert">{error}</div> : null}
        {!draftConfig || !summary ? (
          <section className="panel">Loading</section>
        ) : (
          <section className="content-band">
            {activeTab === "overview" ? <Overview summary={summary} config={draftConfig} status={status} /> : null}
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
                config={draftConfig}
                setClassification={setClassification}
                setError={setError}
              />
            ) : null}
            {activeTab === "mailboxes" ? (
              <Mailboxes config={draftConfig} setConfig={applyDraftConfig} />
            ) : null}
            {activeTab === "mcp" ? (
              <McpServers config={draftConfig} setConfig={applyDraftConfig} />
            ) : null}
            {activeTab === "safety" ? (
              <Safety config={draftConfig} setConfig={applyDraftConfig} />
            ) : null}
            {activeTab === "settings" ? (
              <SettingsPanel config={draftConfig} setConfig={applyDraftConfig} />
            ) : null}
          </section>
        )}
      </main>
      {navOpen ? (
        <button
          aria-label="Close navigation overlay"
          className="nav-scrim"
          type="button"
          onClick={() => setNavOpen(false)}
        />
      ) : null}
      {pendingGuardRequest ? (
        <ConfirmDialog
          cancelLabel="Keep editing"
          confirmLabel="Discard and continue"
          danger
          onCancel={() => setPendingGuardRequest(null)}
          onConfirm={continueAfterDiscarding}
          title="Unsaved config changes"
        >
          <p>
            {pendingGuardRequest.action === "refresh"
              ? "Refreshing will replace the draft config with the latest server copy."
              : "Signing out will leave this browser session and discard the draft config."}
          </p>
          <div className="dialog-actions secondary">
            <button
              className="primary-action"
              disabled={saving}
              type="button"
              onClick={() => {
                continueAfterSaving().catch((cause) => setError(errorMessage(cause)));
              }}
            >
              <Save aria-hidden="true" />
              {saving ? "Saving" : "Save and continue"}
            </button>
          </div>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function configsEqual(first: AppConfig, second: AppConfig): boolean {
  return JSON.stringify(first) === JSON.stringify(second);
}

function topbarSubtitle(
  tab: TabId,
  status: StatusResponse,
  messages: ProcessedEmail[],
  summary: ReturnType<typeof summarizeConfig> | null
): string {
  if (tab === "history") {
    return `${messages.length} processed email${messages.length === 1 ? "" : "s"} loaded`;
  }
  if (tab === "rules") {
    return "Classification labels and mailbox actions";
  }
  if (tab === "mailboxes" && summary) {
    return `${summary.enabledMailboxes}/${summary.mailboxCount} enabled mailboxes`;
  }
  if (tab === "mcp" && summary) {
    return `${summary.mcpServerCount} configured MCP server${summary.mcpServerCount === 1 ? "" : "s"}`;
  }
  if (tab === "safety" && summary) {
    return `${summary.bannedSenderCount} banned sender${summary.bannedSenderCount === 1 ? "" : "s"}`;
  }
  if (tab === "settings") {
    return "Database, AI, prompts, and logging";
  }
  return `${status.enabled_mailboxes} enabled mailbox${status.enabled_mailboxes === 1 ? "" : "es"}`;
}
