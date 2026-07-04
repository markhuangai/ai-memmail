import { useState } from "react";
import { FileText, Save } from "lucide-react";
import { loadPromptFile, savePromptFile } from "../api";
import type { AppConfig } from "../types";
import { errorMessage } from "../viewUtils";

export function SettingsPanel({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const [selectedPrompt, setSelectedPrompt] = useState<{ label: string; path: string } | null>(null);
  const [promptContent, setPromptContent] = useState("");
  const [promptError, setPromptError] = useState("");
  const [promptStatus, setPromptStatus] = useState("");
  const [promptLoading, setPromptLoading] = useState(false);
  const [promptSaving, setPromptSaving] = useState(false);

  async function openPrompt(label: string, path: string) {
    setSelectedPrompt({ label, path });
    setPromptContent("");
    setPromptError("");
    setPromptStatus("");
    setPromptLoading(true);
    try {
      const prompt = await loadPromptFile(path);
      setPromptContent(prompt.content);
    } catch (cause) {
      setPromptError(errorMessage(cause));
    } finally {
      setPromptLoading(false);
    }
  }

  async function savePrompt() {
    if (!selectedPrompt) {
      return;
    }
    setPromptError("");
    setPromptStatus("");
    setPromptSaving(true);
    try {
      const prompt = await savePromptFile(selectedPrompt.path, promptContent);
      setPromptContent(prompt.content);
      setPromptStatus("Prompt saved.");
    } catch (cause) {
      setPromptError(errorMessage(cause));
    } finally {
      setPromptSaving(false);
    }
  }

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
        <PromptPathField
          label="Safety prompt"
          onChange={(value) =>
            setConfig({
              ...config,
              prompts: { ...config.prompts, safety_scan: value }
            })
          }
          onOpen={() => openPrompt("Safety prompt", config.prompts.safety_scan)}
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
          onOpen={() => openPrompt("Classifier prompt", config.prompts.email_classifier)}
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
          onOpen={() => openPrompt("Rule action prompt", config.prompts.rule_action)}
          value={config.prompts.rule_action}
        />
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
          onOpen={() => openPrompt("Outbound review prompt", config.ai.review.prompt_path)}
          value={config.ai.review.prompt_path}
        />
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
      {selectedPrompt ? (
        <div className="prompt-editor">
          <div className="panel-heading">
            <div>
              <h2>{selectedPrompt.label}</h2>
              <p>{selectedPrompt.path}</p>
            </div>
            <button
              disabled={promptLoading || promptSaving}
              type="button"
              onClick={savePrompt}
            >
              <Save aria-hidden="true" />
              Save prompt
            </button>
          </div>
          {promptError ? <p role="alert" className="banner">{promptError}</p> : null}
          {promptStatus ? <p className="success-note">{promptStatus}</p> : null}
          <textarea
            aria-label={`${selectedPrompt.label} content`}
            disabled={promptLoading || promptSaving}
            value={promptLoading ? "Loading prompt..." : promptContent}
            onChange={(event) => setPromptContent(event.target.value)}
          />
        </div>
      ) : null}
    </section>
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
