import { Plus, Trash2 } from "lucide-react";
import { addMcpServer, listToText, removeMcpServer, setListFromText, updateMcpServer } from "../configModel";
import type { AppConfig } from "../types";
import { isSensitiveEnvName, nextEnvKey } from "../viewUtils";

export function McpServers({
  config,
  setConfig
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
}) {
  const servers = Object.entries(config.mcp_servers);

  function patchServer(name: string, updater: (server: AppConfig["mcp_servers"][string]) => AppConfig["mcp_servers"][string]) {
    setConfig(updateMcpServer(config, name, updater));
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
                  patchServer(name, (next) => ({
                    ...next,
                    transport: event.target.value as AppConfig["mcp_servers"][string]["transport"]
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
          <div className="env-editor">
            <div className="env-editor-heading">
              <h3>Env</h3>
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
      ))}
    </div>
  );
}
