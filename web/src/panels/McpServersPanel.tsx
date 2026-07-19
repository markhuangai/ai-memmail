import { useEffect, useMemo, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import {
  addMcpServer,
  listToText,
  removeMcpServer,
  renameMcpServer,
  setListFromText,
  updateMcpServer
} from "../configModel";
import type { AppConfig } from "../types";
import { isSensitiveEnvName, nextEnvKey } from "../viewUtils";
import { ConfirmDialog } from "./ConfirmDialog";

type McpServerConfig = AppConfig["mcp_servers"][string];

export function McpServers({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const servers = Object.entries(config.mcp_servers);
  const serverNames = servers.map(([name]) => name);
  const [selectedName, setSelectedName] = useState(serverNames[0] ?? "");
  const [draftNames, setDraftNames] = useState<Record<string, string>>({});
  const [removeTarget, setRemoveTarget] = useState<string | null>(null);
  const selectedServer = useMemo(
    () => config.mcp_servers[selectedName] ? [selectedName, config.mcp_servers[selectedName]] as const : servers[0] ?? null,
    [config.mcp_servers, selectedName, servers]
  );

  useEffect(() => {
    setDraftNames((current) => {
      const next = Object.fromEntries(
        serverNames.map((name) => [name, current[name] ?? name])
      );
      return sameRecord(current, next) ? current : next;
    });
  }, [serverNames.join("\n")]);

  useEffect(() => {
    if (serverNames.length === 0) {
      setSelectedName("");
      return;
    }
    if (!serverNames.includes(selectedName)) {
      setSelectedName(serverNames[0]);
    }
  }, [selectedName, serverNames]);

  function patchServer(name: string, updater: (server: McpServerConfig) => McpServerConfig) {
    setConfig(updateMcpServer(config, name, updater));
  }

  function setDraftName(name: string, value: string) {
    setDraftNames((current) => ({ ...current, [name]: value }));
  }

  function commitServerName(name: string) {
    const nextName = (draftNames[name] ?? name).trim();
    const nextConfig = renameMcpServer(config, name, nextName);
    if (nextConfig === config) {
      setDraftName(name, name);
      return;
    }
    setDraftNames((current) => {
      const { [name]: _renamed, ...remaining } = current;
      return { ...remaining, [nextName]: nextName };
    });
    setSelectedName(nextName);
    setConfig(nextConfig);
  }

  function addAndSelectServer() {
    const nextConfig = addMcpServer(config);
    const nextName = Object.keys(nextConfig.mcp_servers).find(
      (name) => !Object.prototype.hasOwnProperty.call(config.mcp_servers, name)
    ) ?? Object.keys(nextConfig.mcp_servers)[0] ?? "";
    setConfig(nextConfig);
    setSelectedName(nextName);
  }

  function confirmRemoveServer(name: string) {
    const nextConfig = removeMcpServer(config, name);
    setConfig(nextConfig);
    setRemoveTarget(null);
    setSelectedName(Object.keys(nextConfig.mcp_servers)[0] ?? "");
  }

  function addEnvVariable(name: string) {
    const server = config.mcp_servers[name];
    if (!server) {
      return;
    }
    const key = nextEnvKey(server.env);
    patchServer(name, (next) => ({
      ...next,
      env: { ...next.env, [key]: "" }
    }));
  }

  function renameEnvVariable(name: string, oldKey: string, nextKey: string) {
    const trimmedKey = nextKey.trim();
    patchServer(name, (next) => {
      const { [oldKey]: value, ...remainingEnv } = next.env;
      if (!trimmedKey) {
        return { ...next, env: remainingEnv };
      }
      return {
        ...next,
        env: {
          ...remainingEnv,
          [trimmedKey]: value ?? ""
        }
      };
    });
  }

  function setEnvVariableValue(name: string, key: string, value: string) {
    patchServer(name, (next) => ({
      ...next,
      env: { ...next.env, [key]: value }
    }));
  }

  function removeEnvVariable(name: string, key: string) {
    patchServer(name, (next) => {
      const { [key]: _removed, ...remainingEnv } = next.env;
      return { ...next, env: remainingEnv };
    });
  }

  return (
    <div className="entity-layout mcp-layout">
      <section className="panel entity-list-panel">
        <div className="panel-heading">
          <div>
            <h2>MCP servers</h2>
            <p>{servers.length} configured</p>
          </div>
          <button type="button" onClick={addAndSelectServer}>
            <Plus aria-hidden="true" />
            Add server
          </button>
        </div>
        {servers.length === 0 ? (
          <p className="empty-state">No MCP servers configured</p>
        ) : (
          <div className="entity-list" role="list">
            {servers.map(([name, server]) => (
              <button
                className={selectedServer?.[0] === name ? "entity-row active" : "entity-row"}
                key={name}
                type="button"
                onClick={() => setSelectedName(name)}
              >
                <span>
                  <strong>{name}</strong>
                  <small>{server.transport}</small>
                </span>
                <em>{Object.keys(server.env).length} env</em>
              </button>
            ))}
          </div>
        )}
      </section>

      {selectedServer ? (
        <ServerDetail
          addEnvVariable={addEnvVariable}
          commitServerName={commitServerName}
          draftName={draftNames[selectedServer[0]] ?? selectedServer[0]}
          name={selectedServer[0]}
          patchServer={patchServer}
          removeEnvVariable={removeEnvVariable}
          removeServer={() => setRemoveTarget(selectedServer[0])}
          renameEnvVariable={renameEnvVariable}
          server={selectedServer[1]}
          setDraftName={setDraftName}
          setEnvVariableValue={setEnvVariableValue}
        />
      ) : (
        <section className="panel entity-detail-panel">
          <h2>No MCP server selected</h2>
        </section>
      )}

      {removeTarget ? (
        <ConfirmDialog
          confirmLabel="Remove server"
          danger
          onCancel={() => setRemoveTarget(null)}
          onConfirm={() => confirmRemoveServer(removeTarget)}
          title="Remove MCP server"
        >
          <p>{removeTarget} will be removed from the draft config and mailbox assignments.</p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function ServerDetail({
  addEnvVariable,
  commitServerName,
  draftName,
  name,
  patchServer,
  removeEnvVariable,
  removeServer,
  renameEnvVariable,
  server,
  setDraftName,
  setEnvVariableValue
}: {
  addEnvVariable: (name: string) => void;
  commitServerName: (name: string) => void;
  draftName: string;
  name: string;
  patchServer: (name: string, updater: (server: McpServerConfig) => McpServerConfig) => void;
  removeEnvVariable: (name: string, key: string) => void;
  removeServer: () => void;
  renameEnvVariable: (name: string, oldKey: string, nextKey: string) => void;
  server: McpServerConfig;
  setDraftName: (name: string, value: string) => void;
  setEnvVariableValue: (name: string, key: string, value: string) => void;
}) {
  return (
    <section className="panel entity-detail-panel">
      <div className="panel-heading">
        <div>
          <h2>{name}</h2>
          <p>{server.transport}</p>
        </div>
        <button type="button" onClick={removeServer}>
          <Trash2 aria-hidden="true" />
          Remove
        </button>
      </div>

      <div className="config-section">
        <h3>Connection</h3>
        <div className="form-grid">
          <label>
            Server id
            <input
              value={draftName}
              onBlur={() => commitServerName(name)}
              onChange={(event) => setDraftName(name, event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  event.currentTarget.blur();
                }
              }}
            />
          </label>
          <label>
            Transport
            <select
              value={server.transport}
              onChange={(event) =>
                patchServer(name, (next) => ({
                  ...next,
                  transport: event.target.value as McpServerConfig["transport"]
                }))
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
                patchServer(name, (next) => ({
                  ...next,
                  command: event.target.value || null
                }))
              }
            />
          </label>
          <label>
            URL
            <input
              value={server.url ?? ""}
              onChange={(event) =>
                patchServer(name, (next) => ({
                  ...next,
                  url: event.target.value || null
                }))
              }
            />
          </label>
          <label>
            Args
            <input
              value={listToText(server.args)}
              onChange={(event) =>
                patchServer(name, (next) => ({
                  ...next,
                  args: setListFromText(event.target.value)
                }))
              }
            />
          </label>
        </div>
      </div>

      <div className="env-editor config-section">
        <div className="env-editor-heading">
          <h3>Environment</h3>
          <button type="button" onClick={() => addEnvVariable(name)}>
            <Plus aria-hidden="true" />
            Add variable
          </button>
        </div>
        {Object.entries(server.env).length === 0 ? (
          <p className="muted">No env variables configured.</p>
        ) : (
          <div className="env-list">
            {Object.entries(server.env).map(([key, value]) => (
              <div className="env-row" key={key}>
                <label>
                  Variable
                  <input
                    value={key}
                    onChange={(event) => renameEnvVariable(name, key, event.target.value)}
                  />
                </label>
                <label>
                  Value
                  <input
                    type={isSensitiveEnvName(key) ? "password" : "text"}
                    value={value}
                    onChange={(event) => setEnvVariableValue(name, key, event.target.value)}
                  />
                </label>
                <button
                  aria-label={`Remove ${key}`}
                  type="button"
                  onClick={() => removeEnvVariable(name, key)}
                >
                  <Trash2 aria-hidden="true" />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function sameRecord(first: Record<string, string>, second: Record<string, string>): boolean {
  const firstEntries = Object.entries(first);
  const secondEntries = Object.entries(second);
  return (
    firstEntries.length === secondEntries.length &&
    firstEntries.every(([key, value]) => second[key] === value)
  );
}
