import { type FormEvent, useEffect, useMemo, useState } from "react";
import { Forward, KeyRound, Mail, MessageSquareText, RefreshCw, ShieldAlert, Send } from "lucide-react";
import type { ProcessedEmail } from "../types";
import { formatTimestamp, messageKey, statusPillClass, timestampMs } from "../viewUtils";

export function HistoryPanel({
  canLoadMore,
  messageLimit,
  messages,
  onCreateHandoff,
  onLoadMore
}: {
  canLoadMore: boolean;
  messageLimit: number;
  messages: ProcessedEmail[];
  onCreateHandoff: (message: ProcessedEmail, destination: string) => Promise<void>;
  onLoadMore: () => void;
}) {
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
            <p>{messages.length} loaded / limit {messageLimit}</p>
          </div>
          {canLoadMore ? (
            <button type="button" onClick={onLoadMore}>
              <RefreshCw aria-hidden="true" />
              Load more
            </button>
          ) : null}
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
                <span className="message-row-badges">
                  <span className={statusPillClass(message.status)}>{message.status}</span>
                  <HandoffBadge message={message} />
                </span>
                <span className="message-row-time">{formatTimestamp(message.updated_at)}</span>
              </button>
            );
          })}
        </div>
      </section>
      <MessageDetail
        message={selected}
        onCreateHandoff={onCreateHandoff}
        onSelectMessage={(message) => setSelectedKey(messageKey(message))}
        threadMessages={threadMessages}
      />
    </div>
  );
}

function MessageDetail({
  message,
  onCreateHandoff,
  onSelectMessage,
  threadMessages
}: {
  message: ProcessedEmail;
  onCreateHandoff: (message: ProcessedEmail, destination: string) => Promise<void>;
  onSelectMessage: (message: ProcessedEmail) => void;
  threadMessages: ProcessedEmail[];
}) {
  const [handoffOpen, setHandoffOpen] = useState(false);
  const [destination, setDestination] = useState(message.handoff?.destination ?? "");
  const [submitting, setSubmitting] = useState(false);
  const [handoffError, setHandoffError] = useState("");

  useEffect(() => {
    setDestination(message.handoff?.destination ?? "");
    setHandoffError("");
    setSubmitting(false);
  }, [message]);

  async function submitHandoff(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    setHandoffError("");
    try {
      await onCreateHandoff(message, destination);
      setHandoffOpen(false);
    } catch (cause) {
      setHandoffError(cause instanceof Error ? cause.message : "handoff failed");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <section className="panel message-detail-panel">
      <div className="panel-heading">
        <div>
          <h2>{message.subject || "(no subject)"}</h2>
          <p>{message.from_addr}</p>
        </div>
        <span className="message-detail-badges">
          <span className={statusPillClass(message.status)}>{message.status}</span>
          <HandoffBadge message={message} />
        </span>
      </div>

      <section className="handoff-panel" aria-label="Thread handoff">
        <div>
          <HandoffBadge message={message} />
          {message.handoff ? (
            <p>
              {message.handoff.destination} to {message.handoff.remote_target}
            </p>
          ) : (
            <p>No handoff destination set.</p>
          )}
        </div>
        {handoffOpen ? (
          <form className="handoff-form" onSubmit={submitHandoff}>
            <label>
              Handoff destination
              <input
                type="email"
                value={destination}
                onChange={(event) => setDestination(event.target.value)}
                required
              />
            </label>
            <button type="submit" disabled={submitting}>
              <Forward aria-hidden="true" />
              {submitting ? "Sending" : "Forward chain"}
            </button>
            <button type="button" onClick={() => setHandoffOpen(false)}>
              Cancel
            </button>
            {handoffError ? <p role="alert">{handoffError}</p> : null}
          </form>
        ) : (
          <button type="button" onClick={() => setHandoffOpen(true)}>
            <Forward aria-hidden="true" />
            {message.handoff ? "Forward again" : "Hand off thread"}
          </button>
        )}
      </section>

      <section className="message-section">
        <h3><Mail aria-hidden="true" /> Inbound</h3>
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
        {message.outbound_body_html ? (
          <HtmlOutboundBody
            authoredBody={message.outbound_body}
            htmlBody={message.outbound_body_html}
          />
        ) : message.outbound_body ? (
          <pre className="message-body">{message.outbound_body}</pre>
        ) : message.outbound_body_redacted ? (
          <p className="muted">Forward body omitted because it can include original inbound email content.</p>
        ) : (
          <p className="muted">No outbound body recorded.</p>
        )}
      </section>

      <details className="message-section collapsed-section">
        <summary><ShieldAlert aria-hidden="true" /> Safety and AI</summary>
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
      </details>

      <details className="message-section collapsed-section">
        <summary><MessageSquareText aria-hidden="true" /> Email chain</summary>
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
                <span className="message-row-badges">
                  <span className={statusPillClass(threadMessage.status)}>{threadMessage.status}</span>
                  <HandoffBadge message={threadMessage} />
                </span>
                <span>{formatTimestamp(threadMessage.updated_at)}</span>
              </button>
            );
          })}
        </div>
      </details>

      <details className="message-section collapsed-section">
        <summary><MessageSquareText aria-hidden="true" /> Timeline</summary>
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
      </details>

      <details className="message-section collapsed-section">
        <summary><KeyRound aria-hidden="true" /> Diagnostics</summary>
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
            <dt>In reply to</dt>
            <dd>{message.in_reply_to ?? "none"}</dd>
          </div>
          <div>
            <dt>References</dt>
            <dd>{message.references.length ? message.references.join(", ") : "none"}</dd>
          </div>
          <div>
            <dt>Updated</dt>
            <dd>{formatTimestamp(message.updated_at)}</dd>
          </div>
        </dl>
      </details>
    </section>
  );
}

function HandoffBadge({ message }: { message: ProcessedEmail }) {
  if (!message.handoff) {
    return null;
  }
  const label = handoffLabel(message.handoff.state);
  return <span className={`handoff-chip ${message.handoff.state}`}>{label}</span>;
}

function HtmlOutboundBody({
  authoredBody,
  htmlBody
}: {
  authoredBody?: string | null;
  htmlBody: string;
}) {
  return (
    <div className="html-body-archive">
      <iframe
        className="html-body-preview"
        referrerPolicy="no-referrer"
        sandbox=""
        srcDoc={htmlBody}
        title="Outbound HTML preview"
      />
      {authoredBody ? (
        <div className="text-block">
          <span>Authored text</span>
          <pre className="message-body">{authoredBody}</pre>
        </div>
      ) : null}
      <details className="collapsed-section">
        <summary>HTML source</summary>
        <pre className="message-body">{htmlBody}</pre>
      </details>
    </div>
  );
}

function handoffLabel(state: string): string {
  if (state === "sending") {
    return "Handoff pending";
  }
  if (state === "uncertain") {
    return "Handoff uncertain";
  }
  return "Handed off";
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
