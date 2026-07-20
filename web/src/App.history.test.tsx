import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import {
  sampleClassification,
  sampleConfig,
  sampleConversationDetail,
  sampleConversations,
  sampleMessages
} from "./fixtures";
import type { AppConfig, PortalConversationDetail } from "./types";
import { classificationResponse, jsonResponse } from "./testHelpers";

const jsdomRect: DOMRect = {
  bottom: 0,
  height: 0,
  left: 0,
  right: 0,
  top: 0,
  width: 0,
  x: 0,
  y: 0,
  toJSON: () => ({})
};

const jsdomRectList: DOMRectList = {
  0: jsdomRect,
  length: 1,
  item: (index: number) => (index === 0 ? jsdomRect : null),
  [Symbol.iterator]: () => [jsdomRect][Symbol.iterator]()
};

beforeAll(() => {
  Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => jsdomRect
  });
  Object.defineProperty(Range.prototype, "getClientRects", {
    configurable: true,
    value: () => jsdomRectList
  });
});

describe("App history", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("renders conversation history details", async () => {
    installHistoryMock();

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(await screen.findByRole("heading", { name: "Pricing question" })).toBeInTheDocument();
    expect(screen.getByText("Can you send the current pricing plan?")).toBeInTheDocument();
    expect(screen.getByText("AI reply")).toBeInTheDocument();
    expect(screen.getByLabelText("Decision evidence")).toBeInTheDocument();
    expect(screen.getByText("Quoted conversation")).toBeInTheDocument();
  });

  it("sends a reply to the remote sender with quoted history", async () => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue("33333333-3333-4333-8333-333333333333");
    const requests: unknown[] = [];
    installHistoryMock({ portalRequests: requests });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    fireEvent.change(screen.getByLabelText("HTML source"), {
      target: { value: "<p>I can send more detail.<br><br>--<br>Mark</p>" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toMatchObject({
      request_id: "33333333-3333-4333-8333-333333333333",
      thread_revision: 2,
      action: "reply",
      authored_text: "I can send more detail.\n\n--\nMark",
      unsafe_confirmed: false
    });
  });

  it("requires confirmation before replying to an unsafe conversation", async () => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue("44444444-4444-4444-8444-444444444444");
    const unsafeDetail = unsafeConversationDetail();
    const requests: unknown[] = [];
    installHistoryMock({ details: { [unsafeDetail.conversation.conversation_id]: unsafeDetail }, portalRequests: requests });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Suspicious prompt injection sample/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    fireEvent.change(screen.getByLabelText("HTML source"), {
      target: { value: "<p>I reviewed this.<br><br>--<br>Mark</p>" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    const dialog = await screen.findByRole("dialog", { name: "Reply to unsafe conversation" });
    expect(dialog).toBeInTheDocument();
    expect(requests).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Reply to unsafe conversation" })).not.toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));
    const confirmedDialog = await screen.findByRole("dialog", { name: "Reply to unsafe conversation" });
    fireEvent.click(within(confirmedDialog).getByRole("button", { name: "Send reply" }));
    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toMatchObject({ action: "reply", unsafe_confirmed: true });
  });

  it("sends source-mode HTML", async () => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue("66666666-6666-4666-8666-666666666666");
    const requests: unknown[] = [];
    installHistoryMock({
      config: configWithSignature("html", "<table><tr><td>Mark</td></tr></table>"),
      portalRequests: requests
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(await screen.findByTitle("Read-only message HTML preview")).toHaveAttribute(
      "srcdoc",
      "<p></p><table><tbody><tr><td>Mark</td></tr></tbody></table>"
    );
    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    expect(screen.getByLabelText("HTML source")).toHaveValue("<p></p><table><tr><td>Mark</td></tr></table>");
    fireEvent.change(screen.getByLabelText("HTML source"), {
      target: { value: "<p>I can send<br>details.</p><table><tr><td>Mark</td></tr></table>" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toMatchObject({
      authored_text: "I can send\ndetails.\nMark",
      authored_html: "<p>I can send<br>details.</p><table><tr><td>Mark</td></tr></table>"
    });
  });

  it("preserves configured HTML signature when sending from visual mode", async () => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue("77777777-7777-4777-8777-777777777777");
    const requests: unknown[] = [];
    installHistoryMock({
      config: configWithSignature(
        "html",
        '<p><a href="https://markhuang.ai" target="_blank" rel="noopener noreferrer"><img src="https://markhuang.ai/logo.png" alt="Mark Huang logo" width="58" height="58" style="width: 58px; height: 58px;"></a><br><span style="color: #191d24; font-family: Consolas, \'Liberation Mono\', Menlo, monospace; font-size: 17px;"><strong>Mark Huang</strong></span><br><span style="color: #5c6370; font-family: Arial, Helvetica, sans-serif; font-size: 12px;">AI architect &amp; senior full-stack developer</span><br><a href="https://markhuang.ai" target="_blank" rel="noopener noreferrer"><span style="color: #9c5916; font-family: Arial, Helvetica, sans-serif; font-size: 12px;"><strong>markhuang.ai</strong></span></a><br><br><a href="https://badges.marquiswhoswho.com/Badge/honoredlistee/fba934616c19403ba14cb0df04d2804f448f6eb34f394af498e7a192320a2a3b" target="_blank" rel="noopener noreferrer"><img src="https://badges.marquiswhoswho.com/Badge/honoredlistee/fba934616c19403ba14cb0df04d2804f448f6eb34f394af498e7a192320a2a3b" alt="Marquis Who\'s Who Honored Listee 2026 badge" width="88" height="91" style="width: 88px; height: 91px;"></a></p>'
      ),
      portalRequests: requests
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(await screen.findByRole("toolbar", { name: "Message formatting toolbar" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    await waitFor(() => expect(requests).toHaveLength(1));
    const request = requests[0] as { authored_html?: string; authored_text?: string };
    expect(request.authored_html).toContain('<img src="https://markhuang.ai/logo.png"');
    expect(request.authored_html).toContain('style="color: #191d24; font-family: Consolas, ');
    expect(request.authored_html).toContain('<strong>Mark Huang</strong>');
    expect(request.authored_html).toContain("https://badges.marquiswhoswho.com/Badge/honoredlistee/");
    expect(request.authored_html).not.toContain("&lt;img");
    expect(request.authored_text).toContain("Mark Huang\nAI architect & senior full-stack developer\nmarkhuang.ai");
  });

  it("escapes plain-text signatures when composing HTML", async () => {
    const requests: unknown[] = [];
    installHistoryMock({
      config: configWithSignature("plain_text", "--\nMark <mark@example.com> & Co"),
      portalRequests: requests
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Source" }));
    expect(screen.getByLabelText("HTML source")).toHaveValue(
      "<p></p><p>--<br>Mark &lt;mark@example.com&gt; &amp; Co</p>"
    );
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    await waitFor(() => expect(requests).toHaveLength(1));
    const request = requests[0] as { authored_html?: string; authored_text?: string };
    expect(request.authored_html).toContain("Mark &lt;mark@example.com&gt; &amp; Co");
    expect(request.authored_html).not.toContain("Mark <mark@example.com> & Co");
    expect(request.authored_text).toContain("Mark <mark@example.com> & Co");
  });

  it("sends a normal forward with To Cc and Bcc recipients", async () => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue("55555555-5555-4555-8555-555555555555");
    const requests: unknown[] = [];
    installHistoryMock({ portalRequests: requests });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Forward" }));
    fireEvent.change(screen.getByLabelText("To"), { target: { value: "a@example.com" } });
    fireEvent.change(screen.getByLabelText("Cc"), { target: { value: "c@example.com" } });
    fireEvent.change(screen.getByLabelText("Bcc"), { target: { value: "b@example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Source" }));
    fireEvent.change(screen.getByLabelText("HTML source"), {
      target: { value: "<p>Please review the conversation.</p>" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Send forward" }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toMatchObject({
      action: "forward",
      to_recipients: ["a@example.com"],
      cc_recipients: ["c@example.com"],
      bcc_recipients: ["b@example.com"]
    });
  });

  it("keeps operational handoff separate from normal forward", async () => {
    const handoffRequests: unknown[] = [];
    installHistoryMock({ handoffRequests });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Hand off" }));
    fireEvent.change(screen.getByLabelText(/handoff destination/i), {
      target: { value: "mark.personal@example.com" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Forward chain" }));

    await waitFor(() => expect(handoffRequests).toHaveLength(1));
    expect(handoffRequests[0]).toMatchObject({ destination: "mark.personal@example.com" });
    expect(await screen.findByText("Handed off to mark.personal@example.com")).toBeInTheDocument();
  });

  it("shows detail load errors and disables missing mailbox conversations", async () => {
    const missingMailboxDetail = {
      ...sampleConversationDetail,
      conversation: {
        ...sampleConversationDetail.conversation,
        mailbox_id: "missing"
      }
    };
    installHistoryMock({
      detailFailures: [sampleConversations[1].conversation_id],
      details: {
        [sampleConversationDetail.conversation.conversation_id]: missingMailboxDetail
      }
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(await screen.findByText("Mailbox configuration is missing; replies and forwards are disabled.")).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: /Suspicious prompt injection sample/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("failed to load detail");
  });
});

function installHistoryMock({
  config = sampleConfig,
  detailFailures = [],
  details = {},
  handoffRequests = [],
  portalRequests = []
}: {
  config?: AppConfig;
  detailFailures?: string[];
  details?: Record<string, PortalConversationDetail>;
  handoffRequests?: unknown[];
  portalRequests?: unknown[];
} = {}) {
  const detailMap = {
    [sampleConversationDetail.conversation.conversation_id]: sampleConversationDetail,
    ...details
  };
  vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
    const url = String(path);
    if (url === "/api/status") {
      return Promise.resolve(jsonResponse({
        service: "ai-memmail",
        authenticated: true,
        uptime_seconds: 3,
        enabled_mailboxes: 1
      }));
    }
    if (url.startsWith("/api/conversations/") && url.endsWith("/messages")) {
      portalRequests.push(JSON.parse(String(init?.body)));
      return Promise.resolve(jsonResponse({
        conversation: detailMap[sampleConversationDetail.conversation.conversation_id]
      }));
    }
    if (url.startsWith("/api/conversations/")) {
      const id = decodeURIComponent(url.split("/api/conversations/")[1]);
      if (detailFailures.includes(id)) {
        return Promise.resolve(jsonResponse({ error: "failed to load detail" }, { status: 500 }));
      }
      return Promise.resolve(jsonResponse({ conversation: detailMap[id] ?? sampleConversationDetail }));
    }
    if (url.startsWith("/api/conversations")) {
      return Promise.resolve(jsonResponse({ conversations: sampleConversations }));
    }
    if (url.startsWith("/api/messages/") && init?.method === "POST") {
      handoffRequests.push(JSON.parse(String(init.body)));
      return Promise.resolve(jsonResponse({
        handoff: {
          state: "active",
          destination: "mark.personal@example.com",
          remote_target: "person@example.com",
          last_error: null,
          updated_at: "2026-07-01 00:04:00+00"
        }
      }));
    }
    if (url.startsWith("/api/messages")) {
      return Promise.resolve(jsonResponse({ messages: sampleMessages }));
    }
    if (url === "/api/email-classification") {
      return classificationResponse();
    }
    return Promise.resolve(jsonResponse({ config }));
  });
}

function configWithSignature(format: "html" | "plain_text", content: string): AppConfig {
  return {
    ...sampleConfig,
    mailboxes: [
      {
        ...sampleConfig.mailboxes[0],
        signature: { format, content }
      }
    ]
  };
}

function unsafeConversationDetail(): PortalConversationDetail {
  return {
    conversation: sampleConversations[1],
    quote_text:
      "[1] inbound\nFrom: blocked@example.com\nTo: support@example.com\nSubject: Suspicious prompt injection sample\nMessage-ID: <43@example.com>\n\nRedacted prompt-injection sample requesting instruction override and secret disclosure.",
    quote_html:
      "<section><p><strong>inbound</strong><br>From: blocked@example.com</p><pre>Redacted prompt-injection sample requesting instruction override and secret disclosure.</pre></section>",
    messages: [
      {
        id: `inbound:${sampleMessages[1].run_id}`,
        direction: "inbound",
        kind: "inbound",
        status: "quarantined",
        from_addr: "blocked@example.com",
        to_recipients: ["support@example.com"],
        cc_recipients: [],
        subject: "Suspicious prompt injection sample",
        text_body: sampleMessages[1].inbound_body,
        html_body: null,
        body_truncated: false,
        message_id: "<43@example.com>",
        in_reply_to: null,
        references: [],
        safety_category: "prompt_injection",
        created_at: "2026-07-01 00:03:00+00"
      }
    ]
  };
}
