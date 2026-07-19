import { FormEvent, useEffect, useState } from "react";
import { Plus, Save, Trash2 } from "lucide-react";
import { createEmailCategory, createEmailRule, createEmailTopic, deleteEmailRule, updateEmailRule } from "../api";
import type { AppConfig, EmailClassificationConfig, EmailRule, NewEmailRule } from "../types";
import { errorMessage } from "../viewUtils";
import { ConfirmDialog } from "./ConfirmDialog";

export function RulesPanel({
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
    <div className="stack rules-layout">
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
  const [confirmDelete, setConfirmDelete] = useState(false);

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
    <>
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
              onClick={() => setConfirmDelete(true)}
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
      {confirmDelete && onDelete ? (
        <ConfirmDialog
          confirmLabel="Delete rule"
          danger
          onCancel={() => setConfirmDelete(false)}
          onConfirm={() => {
            setSaving(true);
            onDelete().finally(() => {
              setSaving(false);
              setConfirmDelete(false);
            });
          }}
          title="Delete rule"
        >
          <p>{title ?? draft.name} will be deleted from classification rules.</p>
        </ConfirmDialog>
      ) : null}
    </>
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
