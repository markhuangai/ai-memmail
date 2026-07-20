import { type FormEvent, type ReactNode, useEffect, useMemo, useState } from "react";
import { Forward, Mail, MessageSquareReply, RefreshCw, Send, ShieldAlert } from "lucide-react";
import { newRequestId } from "../api";
import { plainTextToHtml } from "../configModel";
import type {
  AppConfig,
  MailboxConfig,
  PortalConversationDetail,
  PortalConversationSummary,
  PortalSendRequest,
  PortalTimelineMessage
} from "../types";
import { formatTimestamp, statusPillClass } from "../viewUtils";
import { ConfirmDialog } from "./ConfirmDialog";

type ComposerAction = "reply" | "forward";
type HtmlMode = "visual" | "source";

export function HistoryPanel({
  canLoadMore,
  config,
  conversations,
  messageLimit,
  onCreateHandoff,
  onLoadConversation,
  onLoadMore,
  onSendPortalMessage
}: {
  canLoadMore: boolean;
  config: AppConfig;
  conversations: PortalConversationSummary[];
  messageLimit: number;
  onCreateHandoff: (runId: string, destination: string) => Promise<void>;
  onLoadConversation: (conversationId: string) => Promise<PortalConversationDetail>;
  onLoadMore: () => void;
  onSendPortalMessage: (
    conversationId: string,
    request: PortalSendRequest
  ) => Promise<PortalConversationDetail>;
}) {
  const [selectedId, setSelectedId] = useState("");
  const selected = useMemo(
    () =>
      conversations.find((conversation) => conversation.conversation_id === selectedId) ??
      conversations[0] ??
      null,
    [conversations, selectedId]
  );
  const [detail, setDetail] = useState<PortalConversationDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [detailError, setDetailError] = useState("");
  const [handoffNotice, setHandoffNotice] = useState<{
    conversationId: string;
    destination: string;
  } | null>(null);

  useEffect(() => {
    if (!selected) {
      setSelectedId("");
      setDetail(null);
      return;
    }
    if (!selectedId) {
      setSelectedId(selected.conversation_id);
    }
  }, [selected, selectedId]);

  useEffect(() => {
    if (!selected?.conversation_id) {
      return;
    }
    let cancelled = false;
    setLoadingDetail(true);
    setDetailError("");
    onLoadConversation(selected.conversation_id)
      .then((nextDetail) => {
        if (!cancelled) {
          setDetail(nextDetail);
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          setDetail(null);
          setDetailError(cause instanceof Error ? cause.message : "failed to load conversation");
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingDetail(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [onLoadConversation, selected?.conversation_id]);

  if (conversations.length === 0 || !selected) {
    return (
      <section className="panel">
        <h2>No conversations</h2>
      </section>
    );
  }

  return (
    <div className="history-console conversation-workspace">
      <section className="panel message-list-panel history-queue">
        <div className="panel-heading">
          <div>
            <h2>Conversations</h2>
            <p>{conversations.length} loaded / limit {messageLimit}</p>
          </div>
          {canLoadMore ? (
            <button type="button" onClick={onLoadMore}>
              <RefreshCw aria-hidden="true" />
              Load more
            </button>
          ) : null}
        </div>
        <div className="message-list" role="list">
          {conversations.map((conversation) => (
            <button
              className={
                conversation.conversation_id === selected.conversation_id
                  ? "message-row active"
                  : "message-row"
              }
              key={conversation.conversation_id}
              onClick={() => setSelectedId(conversation.conversation_id)}
              type="button"
            >
              <span className="message-row-main">
                <strong>{conversation.subject || "(no subject)"}</strong>
                <span>{conversation.remote_reply_to ?? conversation.latest_sender}</span>
              </span>
              <span className="message-row-badges">
                <span className={statusPillClass(conversation.latest_status)}>
                  {conversation.latest_status}
                </span>
                {conversation.unsafe_reply_requires_confirmation ? (
                  <span className="handoff-chip uncertain">Needs confirm</span>
                ) : null}
              </span>
              <span className="message-row-time">{formatTimestamp(conversation.last_message_at)}</span>
            </button>
          ))}
        </div>
      </section>
      {loadingDetail ? <section className="panel message-detail-panel">Loading</section> : null}
      {!loadingDetail && detail ? (
        <ConversationDetail
          config={config}
          detail={detail}
          handoffNotice={
            handoffNotice?.conversationId === detail.conversation.conversation_id
              ? handoffNotice.destination
              : null
          }
          onHandoffComplete={(destination) =>
            setHandoffNotice({
              conversationId: detail.conversation.conversation_id,
              destination
            })
          }
          onCreateHandoff={onCreateHandoff}
          onReload={() => onLoadConversation(detail.conversation.conversation_id).then(setDetail)}
          onSend={async (request) => {
            const result = await onSendPortalMessage(detail.conversation.conversation_id, request);
            if (request.action === "forward") {
              setDetail(await onLoadConversation(detail.conversation.conversation_id));
              return;
            }
            setDetail(result);
          }}
        />
      ) : null}
      {!loadingDetail && detailError ? (
        <section className="panel message-detail-panel" role="alert">{detailError}</section>
      ) : null}
    </div>
  );
}

function ConversationDetail({
  config,
  detail,
  handoffNotice,
  onCreateHandoff,
  onHandoffComplete,
  onReload,
  onSend
}: {
  config: AppConfig;
  detail: PortalConversationDetail;
  handoffNotice: string | null;
  onCreateHandoff: (runId: string, destination: string) => Promise<void>;
  onHandoffComplete: (destination: string) => void;
  onReload: () => Promise<void>;
  onSend: (request: PortalSendRequest) => Promise<void>;
}) {
  const mailbox = config.mailboxes.find((candidate) => candidate.id === detail.conversation.mailbox_id) ?? null;
  const [composerAction, setComposerAction] = useState<ComposerAction>("reply");
  const [handoffOpen, setHandoffOpen] = useState(false);
  const firstRunId = useMemo(() => firstInboundRunId(detail.messages), [detail.messages]);

  return (
    <>
      <section className="panel message-detail-panel history-correspondence conversation-thread">
        <div className="panel-heading">
          <div>
            <h2>{detail.conversation.subject || "(no subject)"}</h2>
            <p>{detail.conversation.remote_reply_to ?? detail.conversation.latest_sender}</p>
          </div>
          <span className="message-detail-badges">
            <span className={statusPillClass(detail.conversation.latest_status)}>
              {detail.conversation.latest_status}
            </span>
          </span>
        </div>
        <div className="thread-actions">
          <button
            className={composerAction === "reply" ? "active" : ""}
            disabled={!detail.conversation.remote_reply_to}
            onClick={() => setComposerAction("reply")}
            type="button"
          >
            <MessageSquareReply aria-hidden="true" />
            Reply
          </button>
          <button
            className={composerAction === "forward" ? "active" : ""}
            onClick={() => setComposerAction("forward")}
            type="button"
          >
            <Forward aria-hidden="true" />
            Forward
          </button>
          <button type="button" onClick={() => setHandoffOpen((open) => !open)}>
            <Mail aria-hidden="true" />
            Hand off
          </button>
        </div>
        {handoffOpen ? (
          <HandoffForm
            disabled={!firstRunId}
            onCreate={async (destination) => {
              if (!firstRunId) {
                return;
              }
              await onCreateHandoff(firstRunId, destination);
              await onReload();
              onHandoffComplete(destination);
              setHandoffOpen(false);
            }}
          />
        ) : null}
        {handoffNotice ? (
          <p className="handoff-chip" role="status">Handed off to {handoffNotice}</p>
        ) : null}
        <div className="conversation-timeline" role="list">
          {detail.messages.map((message) => (
            <TimelineMessage key={message.id} message={message} />
          ))}
        </div>
        {mailbox ? (
          <PortalComposer
            action={composerAction}
            detail={detail}
            mailbox={mailbox}
            onSend={onSend}
          />
        ) : (
          <p className="muted">Mailbox configuration is missing; replies and forwards are disabled.</p>
        )}
      </section>
      <ConversationEvidence detail={detail} />
    </>
  );
}

function PortalComposer({
  action,
  detail,
  mailbox,
  onSend
}: {
  action: ComposerAction;
  detail: PortalConversationDetail;
  mailbox: MailboxConfig;
  onSend: (request: PortalSendRequest) => Promise<void>;
}) {
  const [mode, setMode] = useState<HtmlMode>("visual");
  const [body, setBody] = useState(() => defaultBody(mailbox));
  const [html, setHtml] = useState(() => defaultHtml(mailbox));
  const [to, setTo] = useState("");
  const [cc, setCc] = useState("");
  const [bcc, setBcc] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const [confirmUnsafe, setConfirmUnsafe] = useState(false);

  useEffect(() => {
    setBody(defaultBody(mailbox));
    setHtml(defaultHtml(mailbox));
    setTo("");
    setCc("");
    setBcc("");
    setError("");
    setConfirmUnsafe(false);
  }, [action, detail.conversation.conversation_id, mailbox]);

  async function submit(unsafeConfirmed = false) {
    setSubmitting(true);
    setError("");
    try {
      const authoredHtml = mode === "source" ? html : plainTextToHtml(body);
      const authoredText = mode === "source" ? htmlToText(html) : body;
      await onSend({
        request_id: newRequestId(),
        thread_revision: detail.conversation.revision,
        action,
        authored_text: authoredText,
        authored_html: authoredHtml,
        to_recipients: action === "forward" ? listFromText(to) : [],
        cc_recipients: action === "forward" ? listFromText(cc) : [],
        bcc_recipients: action === "forward" ? listFromText(bcc) : [],
        unsafe_confirmed: unsafeConfirmed
      });
      setBody(defaultBody(mailbox));
      setHtml(defaultHtml(mailbox));
      setTo("");
      setCc("");
      setBcc("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "send failed");
    } finally {
      setSubmitting(false);
      setConfirmUnsafe(false);
    }
  }

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (action === "reply" && detail.conversation.unsafe_reply_requires_confirmation) {
      setConfirmUnsafe(true);
      return;
    }
    submit().catch(() => undefined);
  }

  return (
    <form className="portal-composer" onSubmit={onSubmit}>
      <div className="composer-head">
        <div>
          <strong>{action === "reply" ? "Reply" : "Forward"}</strong>
          <span>{action === "reply" ? detail.conversation.remote_reply_to : "Choose recipients"}</span>
        </div>
        <div className="segmented-control compact" role="group" aria-label="Composer mode">
          <button className={mode === "visual" ? "active" : ""} type="button" onClick={() => setMode("visual")}>
            Visual
          </button>
          <button className={mode === "source" ? "active" : ""} type="button" onClick={() => setMode("source")}>
            Source
          </button>
        </div>
      </div>
      {action === "forward" ? (
        <div className="recipient-grid">
          <label>To<input value={to} onChange={(event) => setTo(event.target.value)} /></label>
          <label>Cc<input value={cc} onChange={(event) => setCc(event.target.value)} /></label>
          <label>Bcc<input value={bcc} onChange={(event) => setBcc(event.target.value)} /></label>
        </div>
      ) : null}
      {mode === "visual" ? (
        <label>
          Message
          <textarea value={body} onChange={(event) => setBody(event.target.value)} />
        </label>
      ) : (
        <label>
          HTML source
          <textarea value={html} onChange={(event) => setHtml(event.target.value)} />
        </label>
      )}
      <details className="quote-preview" open>
        <summary>Quoted conversation</summary>
        <pre className="message-body">{detail.quote_text}</pre>
      </details>
      <button className="primary-action" disabled={submitting} type="submit">
        <Send aria-hidden="true" />
        {submitting ? "Sending" : action === "reply" ? "Send reply" : "Send forward"}
      </button>
      {error ? <p role="alert" className="signature-error">{error}</p> : null}
      {confirmUnsafe ? (
        <ConfirmDialog
          cancelLabel="Cancel"
          confirmLabel="Send reply"
          danger
          onCancel={() => setConfirmUnsafe(false)}
          onConfirm={() => submit(true).catch(() => undefined)}
          title="Reply to unsafe conversation"
        >
          <p>This reply quotes a quarantined or unsafe stored message.</p>
        </ConfirmDialog>
      ) : null}
    </form>
  );
}

function TimelineMessage({ message }: { message: PortalTimelineMessage }) {
  return (
    <article className={`timeline-message ${message.direction}`} role="listitem">
      <header>
        <span>{messageLabel(message.kind)}</span>
        <time>{formatTimestamp(message.created_at)}</time>
      </header>
      <dl className="detail-grid message-detail-grid compact">
        <div><dt>From</dt><dd>{message.from_addr}</dd></div>
        <div><dt>To</dt><dd>{message.to_recipients.length ? message.to_recipients.join(", ") : "none"}</dd></div>
        <div><dt>Status</dt><dd>{message.status}</dd></div>
        <div><dt>Message ID</dt><dd>{message.message_id ?? "none"}</dd></div>
      </dl>
      {message.html_body ? (
        <iframe
          className="html-body-preview timeline-html"
          referrerPolicy="no-referrer"
          sandbox=""
          srcDoc={message.html_body}
          title={`${message.kind} HTML preview`}
        />
      ) : (
        <pre className="message-body">{message.text_body ?? "No body recorded."}</pre>
      )}
      {message.body_truncated ? <p className="muted">Stored body was truncated.</p> : null}
    </article>
  );
}

function HandoffForm({
  disabled,
  onCreate
}: {
  disabled: boolean;
  onCreate: (destination: string) => Promise<void>;
}) {
  const [destination, setDestination] = useState("");
  const [submitting, setSubmitting] = useState(false);
  return (
    <form
      className="handoff-form"
      onSubmit={(event) => {
        event.preventDefault();
        setSubmitting(true);
        onCreate(destination).finally(() => setSubmitting(false));
      }}
    >
      <label>Handoff destination<input type="email" value={destination} onChange={(event) => setDestination(event.target.value)} required /></label>
      <button disabled={disabled || submitting} type="submit"><Forward aria-hidden="true" />Forward chain</button>
      {disabled ? <p className="muted">No inbound run is available for handoff.</p> : null}
    </form>
  );
}

function ConversationEvidence({ detail }: { detail: PortalConversationDetail }) {
  const unsafe = detail.conversation.unsafe_reply_requires_confirmation;
  return (
    <aside className="panel evidence-panel" aria-label="Decision evidence">
      <div className="panel-heading">
        <div>
          <h2>Conversation evidence</h2>
          <p>{detail.messages.length} messages</p>
        </div>
      </div>
      <EvidenceSection title="Routing" tone="outbound">
        <EvidenceField label="Mailbox" value={detail.conversation.mailbox_id} />
        <EvidenceField label="Thread" value={detail.conversation.thread_id} />
        <EvidenceField label="Revision" value={String(detail.conversation.revision)} />
      </EvidenceSection>
      <EvidenceSection title="Safety" tone={unsafe ? "safe" : "classify"}>
        <EvidenceField label="Reply confirmation" value={unsafe ? "required" : "not required"} />
        <EvidenceField label="Remote reply to" value={detail.conversation.remote_reply_to ?? "none"} />
      </EvidenceSection>
    </aside>
  );
}

function EvidenceSection({
  children,
  title,
  tone
}: {
  children: ReactNode;
  title: string;
  tone: "safe" | "classify" | "outbound";
}) {
  return (
    <section className={`evidence-section ${tone}`}>
      <h3>{title}</h3>
      <dl>{children}</dl>
    </section>
  );
}

function EvidenceField({ label, value }: { label: string; value: string }) {
  return <div><dt>{label}</dt><dd>{value}</dd></div>;
}

function defaultBody(mailbox: MailboxConfig): string {
  if (!mailbox.signature) {
    return "";
  }
  if (mailbox.signature.format === "plain_text") {
    return `\n\n${mailbox.signature.content}`;
  }
  return `\n\n${htmlToText(mailbox.signature.content)}`;
}

function defaultHtml(mailbox: MailboxConfig): string {
  if (!mailbox.signature) {
    return "<p></p>";
  }
  if (mailbox.signature.format === "html") {
    return `<p></p>${mailbox.signature.content}`;
  }
  return `<p></p><pre>${mailbox.signature.content}</pre>`;
}

function htmlToText(html: string): string {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  body.querySelectorAll("br").forEach((element) => element.replaceWith("\n"));
  body.querySelectorAll("p, div, section, article, header, footer, li, table, tr").forEach((element) => {
    element.append("\n");
  });
  return body.textContent
    ?.replace(/[ \t]+\n/g, "\n")
    .replace(/\n[ \t]+/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim() ?? "";
}

function listFromText(value: string): string[] {
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}

function firstInboundRunId(messages: PortalTimelineMessage[]): string | null {
  const inbound = messages.find((message) => message.id.startsWith("inbound:"));
  return inbound?.id.slice("inbound:".length) ?? null;
}

function messageLabel(kind: string): string {
  if (kind === "ai_reply") {
    return "AI reply";
  }
  if (kind === "portal_reply") {
    return "Portal reply";
  }
  if (kind === "portal_forward") {
    return "Portal forward";
  }
  return "Inbound";
}
