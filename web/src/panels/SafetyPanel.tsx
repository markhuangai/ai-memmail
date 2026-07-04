import { useState } from "react";
import { addBannedSender, removeBannedSender } from "../configModel";
import type { AppConfig, BannedSenderConfig } from "../types";

export function Safety({
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
