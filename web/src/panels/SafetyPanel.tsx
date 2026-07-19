import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { addBannedSender, removeBannedSender } from "../configModel";
import type { AppConfig, BannedSenderConfig } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";

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
  const [removeTarget, setRemoveTarget] = useState<BannedSenderConfig | null>(null);

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
        <div className="panel-heading">
          <div>
            <h2>Banned senders</h2>
            <p>{config.banned_senders.length} entries in the draft config</p>
          </div>
        </div>
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
          <button type="button" onClick={addDraft}>
            <Plus aria-hidden="true" />
            Add
          </button>
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
                      onClick={() => setRemoveTarget(sender)}
                    >
                      <Trash2 aria-hidden="true" />
                      Remove
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      {removeTarget ? (
        <ConfirmDialog
          confirmLabel="Remove sender"
          danger
          onCancel={() => setRemoveTarget(null)}
          onConfirm={() => {
            setConfig(removeBannedSender(config, removeTarget));
            setRemoveTarget(null);
          }}
          title="Remove banned sender"
        >
          <p>{removeTarget.value} will be removed from the draft config.</p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}
