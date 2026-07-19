import { useState } from "react";
import { FileText, Save, X } from "lucide-react";
import { loadPromptFile, savePromptFile } from "../api";
import type { AppConfig } from "../types";
import { errorMessage } from "../viewUtils";
import { ConfirmDialog } from "./ConfirmDialog";

type PromptSelection = { label: string; path: string };
type PendingPromptAction =
  | { kind: "open"; prompt: PromptSelection }
  | { kind: "close" };

export function SettingsPanel({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const [selectedPrompt, setSelectedPrompt] = useState<PromptSelection | null>(null);
  const [pendingPromptAction, setPendingPromptAction] = useState<PendingPromptAction | null>(null);
  const [promptContent, setPromptContent] = useState("");
  const [savedPromptContent, setSavedPromptContent] = useState("");
  const [promptError, setPromptError] = useState("");
  const [promptStatus, setPromptStatus] = useState("");
  const [promptLoading, setPromptLoading] = useState(false);
  const [promptSaving, setPromptSaving] = useState(false);
  const promptDirty = Boolean(selectedPrompt && !promptLoading && promptContent !== savedPromptContent);

  function requestOpenPrompt(label: string, path: string) {
    const prompt = { label, path };
    if (promptDirty) {
      setPendingPromptAction({ kind: "open", prompt });
      return;
    }
    loadPrompt(prompt).catch((cause) => setPromptError(errorMessage(cause)));
  }

  async function loadPrompt(prompt: PromptSelection) {
    setSelectedPrompt(prompt);
    setPromptContent("");
    setSavedPromptContent("");
    setPromptError("");
    setPromptStatus("");
    setPromptLoading(true);
    try {
      const loadedPrompt = await loadPromptFile(prompt.path);
      setPromptContent(loadedPrompt.content);
      setSavedPromptContent(loadedPrompt.content);
    } finally {
      setPromptLoading(false);
    }
  }

  async function savePrompt(): Promise<boolean> {
    if (!selectedPrompt) {
      return false;
    }
    setPromptError("");
    setPromptStatus("");
    setPromptSaving(true);
    try {
      const prompt = await savePromptFile(selectedPrompt.path, promptContent);
      setPromptContent(prompt.content);
      setSavedPromptContent(prompt.content);
      setPromptStatus("Prompt saved.");
      return true;
    } catch (cause) {
      setPromptError(errorMessage(cause));
      return false;
    } finally {
      setPromptSaving(false);
    }
  }

  function requestClosePrompt() {
    if (promptDirty) {
      setPendingPromptAction({ kind: "close" });
      return;
    }
    closePrompt();
  }

  function closePrompt() {
    setSelectedPrompt(null);
    setPromptContent("");
    setSavedPromptContent("");
    setPromptError("");
    setPromptStatus("");
  }

  async function saveAndContinuePrompt() {
    const saved = await savePrompt();
    if (!saved || !pendingPromptAction) {
      return;
    }
    const action = pendingPromptAction;
    setPendingPromptAction(null);
    if (action.kind === "open") {
      await loadPrompt(action.prompt);
      return;
    }
    closePrompt();
  }

  function discardAndContinuePrompt() {
    if (!pendingPromptAction) {
      return;
    }
    const action = pendingPromptAction;
    setPendingPromptAction(null);
    if (action.kind === "open") {
      loadPrompt(action.prompt).catch((cause) => setPromptError(errorMessage(cause)));
      return;
    }
    closePrompt();
  }

  return (
    <div className="settings-layout">
      <section className="panel settings-section">
        <div className="panel-heading">
          <div>
            <h2>Database</h2>
            <p>Postgres connection for stored config, history, and classification records.</p>
          </div>
        </div>
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
        </div>
      </section>

      <section className="panel settings-section">
        <div className="panel-heading">
          <div>
            <h2>AI</h2>
            <p>Protocol endpoint, model, and outbound review prompt.</p>
          </div>
        </div>
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
          <PromptPathField
            label="Outbound review prompt"
            onChange={(value) =>
              setConfig({
                ...config,
                ai: {
                  ...config.ai,
                  review: { ...config.ai.review, prompt_path: value }
                }
              })
            }
            onOpen={() => requestOpenPrompt("Outbound review prompt", config.ai.review.prompt_path)}
            value={config.ai.review.prompt_path}
          />
        </div>
      </section>

      <section className="panel settings-section">
        <div className="panel-heading">
          <div>
            <h2>Prompts</h2>
            <p>Config paths save with config; prompt contents save independently.</p>
          </div>
        </div>
        <div className="form-grid">
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
          <PromptPathField
            label="Safety prompt"
            onChange={(value) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, safety_scan: value }
              })
            }
            onOpen={() => requestOpenPrompt("Safety prompt", config.prompts.safety_scan)}
            value={config.prompts.safety_scan}
          />
          <PromptPathField
            label="Classifier prompt"
            onChange={(value) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, email_classifier: value }
              })
            }
            onOpen={() => requestOpenPrompt("Classifier prompt", config.prompts.email_classifier)}
            value={config.prompts.email_classifier}
          />
          <PromptPathField
            label="Rule action prompt"
            onChange={(value) =>
              setConfig({
                ...config,
                prompts: { ...config.prompts, rule_action: value }
              })
            }
            onOpen={() => requestOpenPrompt("Rule action prompt", config.prompts.rule_action)}
            value={config.prompts.rule_action}
          />
        </div>
      </section>

      <section className="panel settings-section">
        <div className="panel-heading">
          <div>
            <h2>Logging</h2>
            <p>Runtime log output and retention.</p>
          </div>
        </div>
        <div className="form-grid">
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

      {selectedPrompt ? (
        <div className="prompt-modal-backdrop" role="presentation">
          <section
            aria-labelledby="prompt-editor-title"
            aria-modal="true"
            className="prompt-editor prompt-modal"
            role="dialog"
          >
            <div className="panel-heading">
              <div>
                <h2 id="prompt-editor-title">{selectedPrompt.label}</h2>
                <p>{selectedPrompt.path}</p>
              </div>
              <div className="panel-actions">
                <button
                  disabled={promptLoading || promptSaving || !promptDirty}
                  type="button"
                  onClick={() => {
                    savePrompt().catch((cause) => setPromptError(errorMessage(cause)));
                  }}
                >
                  <Save aria-hidden="true" />
                  {promptSaving ? "Saving" : "Save prompt"}
                </button>
                <button aria-label="Close prompt" type="button" onClick={requestClosePrompt}>
                  <X aria-hidden="true" />
                </button>
              </div>
            </div>
            {promptError ? <p role="alert" className="banner">{promptError}</p> : null}
            {promptStatus ? <p className="success-note">{promptStatus}</p> : null}
            <textarea
              aria-label={`${selectedPrompt.label} content`}
              disabled={promptLoading || promptSaving}
              value={promptLoading ? "Loading prompt..." : promptContent}
              onChange={(event) => setPromptContent(event.target.value)}
            />
          </section>
        </div>
      ) : null}

      {pendingPromptAction ? (
        <ConfirmDialog
          cancelLabel="Keep editing"
          confirmLabel="Discard prompt"
          danger
          onCancel={() => setPendingPromptAction(null)}
          onConfirm={discardAndContinuePrompt}
          title="Unsaved prompt changes"
        >
          <p>The open prompt has edits that are not saved to its prompt file.</p>
          <div className="dialog-actions secondary">
            <button
              className="primary-action"
              disabled={promptSaving}
              type="button"
              onClick={() => {
                saveAndContinuePrompt().catch((cause) => setPromptError(errorMessage(cause)));
              }}
            >
              <Save aria-hidden="true" />
              {promptSaving ? "Saving" : "Save prompt"}
            </button>
          </div>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function PromptPathField({
  label,
  onChange,
  onOpen,
  value
}: {
  label: string;
  onChange: (value: string) => void;
  onOpen: () => void;
  value: string;
}) {
  return (
    <div className="prompt-path-field">
      <label>
        {label}
        <input value={value} onChange={(event) => onChange(event.target.value)} />
      </label>
      <button aria-label={`Open ${label}`} type="button" onClick={onOpen}>
        <FileText aria-hidden="true" />
      </button>
    </div>
  );
}
