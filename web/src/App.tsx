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
  Tags,
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
import {
  createEmailCategory,
  createEmailRule,
  createEmailTopic,
  deleteEmailRule,
  loadConfig,
  loadEmailClassification,
  loadMessages,
  loadStatus,
  login,
  logout,
  saveConfig,
  updateEmailRule
} from "./api";
import type {
  AppConfig,
  BannedSenderConfig,
  EmailClassificationConfig,
  EmailRule,
  MailboxConfig,
  NewEmailRule,
  ProcessedEmail,
  StatusResponse
} from "./types";

type TabId = "overview" | "history" | "rules" | "mailboxes" | "mcp" | "safety" | "settings";

const tabs: Array<{ id: TabId; label: string; icon: typeof Activity }> = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "history", label: "History", icon: HistoryIcon },
  { id: "rules", label: "Rules", icon: Tags },
  { id: "mailboxes", label: "Mailboxes", icon: Mail },
  { id: "mcp", label: "MCP Servers", icon: Server },
  { id: "safety", label: "Safety", icon: ShieldAlert },
  { id: "settings", label: "Settings", icon: Settings }
];

export function App() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [messages, setMessages] = useState<ProcessedEmail[]>([]);
  const [classification, setClassification] = useState<EmailClassificationConfig | null>(null);
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
          <div>
            <dt>Category</dt>
            <dd>{message.classification_category ?? "not classified"}</dd>
          </div>
          <div>
            <dt>Topics</dt>
            <dd>{message.classification_topics.length ? message.classification_topics.join(", ") : "none"}</dd>
          </div>
          <div>
            <dt>Decision source</dt>
            <dd>{message.decision_source ?? "not recorded"}</dd>
          </div>
          <div>
            <dt>Matched rule</dt>
            <dd>{message.matched_rule_name ?? "none"}</dd>
          </div>
        </dl>
        <TextBlock label="Classification reason" value={message.classification_reason} />
        <TextBlock label="Matched rule goal" value={message.matched_rule_goal} />
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

function RulesPanel({
  classification,
  config,
  setClassification,
  setError
}: {
  classification: EmailClassificationConfig | null;
  config: AppConfig;
  setClassification: (classification: EmailClassificationConfig) => void;
  setError: (message: string) => void;
}) {
  if (!classification) {
    return (
      <section className="panel">
        <h2>Loading rules</h2>
      </section>
    );
  }

  const activeCategories = classification.categories.filter((category) => category.status === "active");
  const activeTopics = classification.topics.filter((topic) => topic.status === "active");

  return (
    <div className="stack">
      <div className="rules-grid">
        <LabelCreator
          buttonLabel="Add category"
          onCreate={async (name, description) => {
            try {
              setError("");
              setClassification(await createEmailCategory(name, description));
            } catch (cause) {
              setError(errorMessage(cause));
            }
          }}
          title="Categories"
        />
        <LabelCreator
          buttonLabel="Add topic"
          onCreate={async (name, description) => {
            try {
              setError("");
              setClassification(await createEmailTopic(name, description));
            } catch (cause) {
              setError(errorMessage(cause));
            }
          }}
          title="Topics"
        />
      </div>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <h2>New Rule</h2>
            <p>Rules match category first, then optional topics.</p>
          </div>
        </div>
        <RuleForm
          categories={activeCategories}
          config={config}
          mode="create"
          onDelete={null}
          onSave={async (rule) => {
            try {
              setError("");
              setClassification(await createEmailRule(rule));
            } catch (cause) {
              setError(errorMessage(cause));
            }
          }}
          rule={createRuleDraft(config, classification)}
          topics={activeTopics}
        />
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <h2>Mailbox Rules</h2>
            <p>{classification.rules.length} active and archived rules</p>
          </div>
        </div>
        {classification.rules.length === 0 ? (
          <p className="muted">No rules configured.</p>
        ) : (
          <div className="rule-list">
            {classification.rules.map((rule) => (
              <RuleForm
                categories={activeCategories}
                config={config}
                key={rule.id}
                mode="edit"
                onDelete={async () => {
                  try {
                    setError("");
                    setClassification(await deleteEmailRule(rule.id));
                  } catch (cause) {
                    setError(errorMessage(cause));
                  }
                }}
                onSave={async (draft) => {
                  try {
                    setError("");
                    setClassification(await updateEmailRule(rule.id, draft));
                  } catch (cause) {
                    setError(errorMessage(cause));
                  }
                }}
                rule={ruleToDraft(rule)}
                title={rule.name}
                topics={activeTopics}
              />
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <h2>Current Labels</h2>
        <div className="label-cloud" aria-label="Configured categories and topics">
          {classification.categories.map((category) => (
            <span className="label-chip" key={`category:${category.id}`}>
              category:{category.name}
            </span>
          ))}
          {classification.topics.map((topic) => (
            <span className="label-chip" key={`topic:${topic.id}`}>
              topic:{topic.name}
            </span>
          ))}
        </div>
      </section>
    </div>
  );
}

function LabelCreator({
  buttonLabel,
  onCreate,
  title
}: {
  buttonLabel: string;
  onCreate: (name: string, description: string) => Promise<void>;
  title: string;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!name.trim()) {
      return;
    }
    setSaving(true);
    try {
      await onCreate(name.trim(), description.trim());
      setName("");
      setDescription("");
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="panel">
      <h2>{title}</h2>
      <form className="label-form" onSubmit={submit}>
        <label>
          Name
          <input value={name} onChange={(event) => setName(event.target.value)} />
        </label>
        <label>
          Description
          <textarea value={description} onChange={(event) => setDescription(event.target.value)} />
        </label>
        <button type="submit" disabled={saving || !name.trim()}>
          <Plus aria-hidden="true" />
          {saving ? "Adding" : buttonLabel}
        </button>
      </form>
    </section>
  );
}

function RuleForm({
  categories,
  config,
  mode,
  onDelete,
  onSave,
  rule,
  title,
  topics
}: {
  categories: EmailClassificationConfig["categories"];
  config: AppConfig;
  mode: "create" | "edit";
  onDelete: (() => Promise<void>) | null;
  onSave: (rule: NewEmailRule) => Promise<void>;
  rule: NewEmailRule;
  title?: string;
  topics: EmailClassificationConfig["topics"];
}) {
  const [draft, setDraft] = useState<NewEmailRule>(rule);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDraft(rule);
  }, [rule.mailbox_id, rule.name, rule.category_id, rule.action, rule.reply_goal, rule.enabled, rule.priority, rule.topic_ids.join(":")]);

  async function save(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    try {
      await onSave(draft);
      if (mode === "create") {
        setDraft(createRuleDraft(config, {
          categories,
          topics
        }));
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <form className={mode === "edit" ? "rule-editor" : "rule-editor new"} onSubmit={save}>
      {title ? <h3>{title}</h3> : null}
      <div className="form-grid">
        <label>
          Rule name
          <input
            value={draft.name}
            onChange={(event) => setDraft({ ...draft, name: event.target.value })}
          />
        </label>
        <label>
          Mailbox
          <select
            value={draft.mailbox_id}
            onChange={(event) => setDraft({ ...draft, mailbox_id: event.target.value })}
          >
            {config.mailboxes.map((mailbox) => (
              <option key={mailbox.id} value={mailbox.id}>
                {mailbox.id}
              </option>
            ))}
          </select>
        </label>
        <label>
          Category
          <select
            value={draft.category_id}
            onChange={(event) => setDraft({ ...draft, category_id: Number(event.target.value) })}
          >
            {categories.map((category) => (
              <option key={category.id} value={category.id}>
                {category.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Action
          <select
            value={draft.action}
            onChange={(event) =>
              setDraft({ ...draft, action: event.target.value as NewEmailRule["action"] })
            }
          >
            <option value="reply">reply</option>
            <option value="forward">forward</option>
            <option value="noop">noop</option>
          </select>
        </label>
        <label>
          Priority
          <input
            min="1"
            type="number"
            value={draft.priority}
            onChange={(event) => setDraft({ ...draft, priority: Number(event.target.value) })}
          />
        </label>
        <label className="switch">
          <input
            checked={draft.enabled}
            type="checkbox"
            onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })}
          />
          Enabled
        </label>
      </div>
      <fieldset className="topic-picker">
        <legend>Topics</legend>
        <p className="muted">No selected topics means any topic in the category.</p>
        <div>
          {topics.map((topic) => (
            <label className="switch" key={topic.id}>
              <input
                checked={draft.topic_ids.includes(topic.id)}
                type="checkbox"
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    topic_ids: toggleTopicId(draft.topic_ids, topic.id, event.target.checked)
                  })
                }
              />
              {topic.name}
            </label>
          ))}
        </div>
      </fieldset>
      <label>
        Response goal
        <textarea
          value={draft.reply_goal}
          onChange={(event) => setDraft({ ...draft, reply_goal: event.target.value })}
        />
      </label>
      <div className="panel-actions">
        {onDelete ? (
          <button
            type="button"
            onClick={() => {
              setSaving(true);
              onDelete().finally(() => setSaving(false));
            }}
            disabled={saving}
          >
            <Trash2 aria-hidden="true" />
            Delete
          </button>
        ) : null}
        <button
          type="submit"
          disabled={saving || !draft.name.trim() || draft.category_id === 0 || !draft.mailbox_id}
        >
          <Save aria-hidden="true" />
          {saving ? "Saving" : mode === "create" ? "Add rule" : "Save rule"}
        </button>
      </div>
    </form>
  );
}

function createRuleDraft(
  config: AppConfig,
  classification: Pick<EmailClassificationConfig, "categories" | "topics">
): NewEmailRule {
  const category = classification.categories.find((candidate) => candidate.status === "active");
  return {
    mailbox_id: config.mailboxes[0]?.id ?? "",
    name: "",
    category_id: category?.id ?? 0,
    topic_ids: [],
    action: "reply",
    reply_goal: "",
    enabled: true,
    priority: 100
  };
}

function ruleToDraft(rule: EmailRule): NewEmailRule {
  return {
    mailbox_id: rule.mailbox_id,
    name: rule.name,
    category_id: rule.category_id,
    topic_ids: rule.topic_ids,
    action: rule.action,
    reply_goal: rule.reply_goal,
    enabled: rule.enabled,
    priority: rule.priority
  };
}

function toggleTopicId(topicIds: number[], topicId: number, checked: boolean) {
  if (checked) {
    return Array.from(new Set([...topicIds, topicId])).sort((a, b) => a - b);
  }
  return topicIds.filter((candidate) => candidate !== topicId);
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
          Classifier prompt
          <input
            value={config.prompts.email_classifier}
            onChange={(event) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, email_classifier: event.target.value }
              })
            }
          />
        </label>
        <label>
          Rule action prompt
          <input
            value={config.prompts.rule_action}
            onChange={(event) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, rule_action: event.target.value }
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
