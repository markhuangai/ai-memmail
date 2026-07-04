import { Plus, Trash2 } from "lucide-react";
import { addMailbox, listToText, mailboxRouteLabel, removeMailbox, setListFromText, updateMailbox } from "../configModel";
import type { AppConfig, MailboxConfig } from "../types";

export function Mailboxes({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const mcpServerNames = Object.keys(config.mcp_servers);

  function patchMailbox(mailboxId: string, updater: (mailbox: MailboxConfig) => MailboxConfig) {
    setConfig(updateMailbox(config, mailboxId, updater));
  }

  function toggleMailboxMcpServer(mailbox: MailboxConfig, serverName: string, enabled: boolean) {
    patchMailbox(mailbox.id, (next) => ({
      ...next,
      mcp_servers: enabled
        ? Array.from(new Set([...next.mcp_servers, serverName]))
        : next.mcp_servers.filter((candidate) => candidate !== serverName)
    }));
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
