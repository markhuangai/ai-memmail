import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
    fireEvent.change(await screen.findByLabelText("Message"), {
      target: { value: "I can send more detail.\n\n--\nMark" }
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
    fireEvent.change(await screen.findByLabelText("Message"), {
      target: { value: "I reviewed this.\n\n--\nMark" }
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
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "Please review the conversation." }
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
