import { useEffect, useMemo, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import {
  addMailbox,
  listToLines,
  listToText,
  mailboxRouteLabel,
  removeMailbox,
  setLinesFromText,
  setListFromText,
  updateMailbox
} from "../configModel";
import type { AcceptedCondition, AppConfig, MailboxConfig } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";
import { SignatureEditor } from "./SignatureEditor";

export function Mailboxes({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const [selectedId, setSelectedId] = useState(config.mailboxes[0]?.id ?? "");
  const [removeTarget, setRemoveTarget] = useState<MailboxConfig | null>(null);
  const mcpServerNames = Object.keys(config.mcp_servers);
  const selectedMailbox = useMemo(
    () => config.mailboxes.find((mailbox) => mailbox.id === selectedId) ?? config.mailboxes[0] ?? null,
    [config.mailboxes, selectedId]
  );

  useEffect(() => {
    if (config.mailboxes.length === 0) {
      setSelectedId("");
      return;
    }
    if (!config.mailboxes.some((mailbox) => mailbox.id === selectedId)) {
      setSelectedId(config.mailboxes[0].id);
    }
  }, [config.mailboxes, selectedId]);

  function patchMailbox(mailboxId: string, updater: (mailbox: MailboxConfig) => MailboxConfig) {
    setConfig(updateMailbox(config, mailboxId, updater));
  }

  function addAndSelectMailbox() {
    const nextConfig = addMailbox(config);
    const nextMailbox = nextConfig.mailboxes[nextConfig.mailboxes.length - 1];
    setConfig(nextConfig);
    setSelectedId(nextMailbox.id);
  }

  function confirmRemoveMailbox(mailbox: MailboxConfig) {
    const nextConfig = removeMailbox(config, mailbox.id);
    setConfig(nextConfig);
    setRemoveTarget(null);
    setSelectedId(nextConfig.mailboxes[0]?.id ?? "");
  }

  function toggleMailboxMcpServer(mailbox: MailboxConfig, serverName: string, enabled: boolean) {
    patchMailbox(mailbox.id, (next) => ({
      ...next,
      mcp_servers: enabled
        ? Array.from(new Set([...next.mcp_servers, serverName]))
        : next.mcp_servers.filter((candidate) => candidate !== serverName)
    }));
  }

  function addAcceptedCondition(mailbox: MailboxConfig) {
    patchMailbox(mailbox.id, (next) => ({
      ...next,
      accepted_conditions: [
        ...(next.accepted_conditions ?? []),
        { recipients: next.address ? [next.address] : [], subject_regex: [] }
      ]
    }));
  }

  function updateAcceptedCondition(
    mailbox: MailboxConfig,
    index: number,
    updater: (condition: AcceptedCondition) => AcceptedCondition
  ) {
    patchMailbox(mailbox.id, (next) => ({
      ...next,
      accepted_conditions: (next.accepted_conditions ?? []).map((condition, conditionIndex) =>
        conditionIndex === index ? updater(condition) : condition
      )
    }));
  }

  function removeAcceptedCondition(mailbox: MailboxConfig, index: number) {
    patchMailbox(mailbox.id, (next) => ({
      ...next,
      accepted_conditions: (next.accepted_conditions ?? []).filter(
        (_condition, conditionIndex) => conditionIndex !== index
      )
    }));
  }

  return (
    <div className="entity-layout mailbox-layout">
      <section className="panel entity-list-panel">
        <div className="panel-heading">
          <div>
            <h2>Mailboxes</h2>
            <p>{config.mailboxes.length} configured</p>
          </div>
          <button type="button" onClick={addAndSelectMailbox}>
            <Plus aria-hidden="true" />
            Add mailbox
          </button>
        </div>
        {config.mailboxes.length === 0 ? (
          <p className="empty-state">No mailboxes configured</p>
        ) : (
          <div className="entity-list" role="list">
            {config.mailboxes.map((mailbox) => (
              <button
                className={selectedMailbox?.id === mailbox.id ? "entity-row active" : "entity-row"}
                key={mailbox.id}
                type="button"
                onClick={() => setSelectedId(mailbox.id)}
              >
                <span>
                  <strong>{mailbox.address}</strong>
                  <small>{mailbox.id}</small>
                </span>
                <em>{mailbox.enabled ? "enabled" : "disabled"}</em>
              </button>
            ))}
          </div>
        )}
      </section>

      {selectedMailbox ? (
        <MailboxDetail
          addAcceptedCondition={addAcceptedCondition}
          mailbox={selectedMailbox}
          mcpServerNames={mcpServerNames}
          patchMailbox={patchMailbox}
          removeAcceptedCondition={removeAcceptedCondition}
          removeMailbox={() => setRemoveTarget(selectedMailbox)}
          toggleMailboxMcpServer={toggleMailboxMcpServer}
          updateAcceptedCondition={updateAcceptedCondition}
        />
      ) : (
        <section className="panel entity-detail-panel">
          <h2>No mailbox selected</h2>
        </section>
      )}

      {removeTarget ? (
        <ConfirmDialog
          confirmLabel="Remove mailbox"
          danger
          onCancel={() => setRemoveTarget(null)}
          onConfirm={() => confirmRemoveMailbox(removeTarget)}
          title="Remove mailbox"
        >
          <p>{removeTarget.address} will be removed from the draft config.</p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function MailboxDetail({
  addAcceptedCondition,
  mailbox,
  mcpServerNames,
  patchMailbox,
  removeAcceptedCondition,
  removeMailbox,
  toggleMailboxMcpServer,
  updateAcceptedCondition
}: {
  addAcceptedCondition: (mailbox: MailboxConfig) => void;
  mailbox: MailboxConfig;
  mcpServerNames: string[];
  patchMailbox: (mailboxId: string, updater: (mailbox: MailboxConfig) => MailboxConfig) => void;
  removeAcceptedCondition: (mailbox: MailboxConfig, index: number) => void;
  removeMailbox: () => void;
  toggleMailboxMcpServer: (mailbox: MailboxConfig, serverName: string, enabled: boolean) => void;
  updateAcceptedCondition: (
    mailbox: MailboxConfig,
    index: number,
    updater: (condition: AcceptedCondition) => AcceptedCondition
  ) => void;
}) {
  return (
    <section className="panel entity-detail-panel">
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
          <button type="button" onClick={removeMailbox}>
            <Trash2 aria-hidden="true" />
            Remove
          </button>
        </div>
      </div>

      <div className="config-section">
        <h3>Identity and routing</h3>
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
          <fieldset className="checkbox-panel">
            <legend>MCP servers</legend>
            {mcpServerNames.length === 0 ? (
              <p className="muted">No MCP servers configured.</p>
            ) : (
              <div className="checkbox-list">
                {mcpServerNames.map((serverName) => (
                  <label className="switch" key={serverName}>
                    <input
                      aria-label={`MCP server ${serverName}`}
                      checked={mailbox.mcp_servers.includes(serverName)}
                      onChange={(event) =>
                        toggleMailboxMcpServer(mailbox, serverName, event.target.checked)
                      }
                      type="checkbox"
                    />
                    {serverName}
                  </label>
                ))}
              </div>
            )}
          </fieldset>
        </div>
      </div>

      <fieldset className="checkbox-panel accepted-conditions">
        <legend>Accepted conditions</legend>
        {(mailbox.accepted_conditions ?? []).length === 0 ? (
          <p className="muted">All unseen messages are eligible.</p>
        ) : (
          <div className="condition-list">
            {(mailbox.accepted_conditions ?? []).map((condition, index) => (
              <div className="condition-row" key={index}>
                <label>
                  Recipients
                  <input
                    value={listToText(condition.recipients)}
                    onChange={(event) =>
                      updateAcceptedCondition(mailbox, index, (next) => ({
                        ...next,
                        recipients: setListFromText(event.target.value)
                      }))
                    }
                  />
                </label>
                <label>
                  Subject regex
                  <textarea
                    value={listToLines(condition.subject_regex)}
                    onChange={(event) =>
                      updateAcceptedCondition(mailbox, index, (next) => ({
                        ...next,
                        subject_regex: setLinesFromText(event.target.value)
                      }))
                    }
                  />
                </label>
                <button
                  aria-label={`Remove accepted condition ${index + 1}`}
                  type="button"
                  onClick={() => removeAcceptedCondition(mailbox, index)}
                >
                  <Trash2 aria-hidden="true" />
                </button>
              </div>
            ))}
          </div>
        )}
        <button type="button" onClick={() => addAcceptedCondition(mailbox)}>
          <Plus aria-hidden="true" />
          Add condition
        </button>
      </fieldset>

      <div className="config-section">
        <h3>IMAP</h3>
        <div className="form-grid">
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
          <label>
            IMAP Sent folder override
            <input
              placeholder="Auto-discover \\Sent"
              value={mailbox.imap.sent_folder ?? ""}
              onChange={(event) =>
                patchMailbox(mailbox.id, (next) => ({
                  ...next,
                  imap: {
                    ...next.imap,
                    sent_folder: event.target.value.trim() || null
                  }
                }))
              }
            />
          </label>
          <label>
            Sent backfill days
            <input
              type="number"
              min="0"
              max="65535"
              value={mailbox.imap.sent_backfill_days}
              onChange={(event) =>
                patchMailbox(mailbox.id, (next) => ({
                  ...next,
                  imap: {
                    ...next.imap,
                    sent_backfill_days: Number(event.target.value)
                  }
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
        </div>
      </div>

      <div className="config-section">
        <h3>SMTP</h3>
        <div className="form-grid">
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
        </div>
      </div>

      <div className="config-section">
        <h3>Agent</h3>
        <div className="form-grid">
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
      </div>

      <SignatureEditor
        key={mailbox.id}
        mailbox={mailbox}
        onChange={(signature) =>
          patchMailbox(mailbox.id, (next) => ({
            ...next,
            signature
          }))
        }
      />
    </section>
  );
}
