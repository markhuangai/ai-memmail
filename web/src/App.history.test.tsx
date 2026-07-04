import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleConfig, sampleMessages } from "./fixtures";
import { classificationResponse, jsonResponse } from "./testHelpers";

describe("App history", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("renders processed email history details", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: sampleMessages });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(screen.getByRole("heading", { name: "Pricing question" })).toBeInTheDocument();
    expect(screen.getAllByText("person@example.com").length).toBeGreaterThan(0);
    expect(screen.getByText("Can you send the current pricing plan?")).toBeInTheDocument();
    expect(screen.getByText(/This is an automated email reply/i)).toBeInTheDocument();
    expect(screen.getByText("<auto-42@example.com>")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Suspicious prompt injection sample/i }));
    expect(screen.getByText("Redacted prompt-injection sample requesting instruction override and secret disclosure.")).toBeInTheDocument();
    expect(screen.getByText(/Forward body omitted/i)).toBeInTheDocument();
    fireEvent.click(screen.getByText("Safety and AI"));
    expect(screen.getAllByText("prompt_injection").length).toBeGreaterThan(0);
  });

  it("renders empty processed email history", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(screen.getByRole("heading", { name: "No processed messages" })).toBeInTheDocument();
  });

  it("loads more processed email history when the current limit is full", async () => {
    const firstBatch = Array.from({ length: 100 }, (_, index) => ({
      ...sampleMessages[0],
      run_id: `run-${index}`,
      uid: index + 1,
      thread_id: `<${index + 1}@example.com>`,
      message_id: `<${index + 1}@example.com>`,
      subject: `Message ${index + 1}`
    }));
    const secondBatch = [
      ...firstBatch,
      {
        ...sampleMessages[0],
        run_id: "run-101",
        uid: 101,
        thread_id: "<101@example.com>",
        message_id: "<101@example.com>",
        subject: "Message 101"
      }
    ];
    const messageRequests: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        messageRequests.push(String(path));
        return jsonResponse({
          messages: messageRequests.length === 1 ? firstBatch : secondBatch
        });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    fireEvent.click(await screen.findByRole("button", { name: /load more/i }));

    await waitFor(() => expect(messageRequests).toEqual([
      "/api/messages?limit=100",
      "/api/messages?limit=200"
    ]));
    expect(screen.getByRole("button", { name: /Message 101/i })).toBeInTheDocument();
  });

  it("links processed messages in the same email chain", async () => {
    const followUp = {
      ...sampleMessages[0],
      run_id: "7a6a8c50-51f5-4e3d-bf9d-a75d0083ec60",
      uid: 44,
      message_id: "<44@example.com>",
      in_reply_to: "<auto-42@example.com>",
      references: ["<42@example.com>", "<auto-42@example.com>"],
      subject: "Re: Pricing question",
      inbound_body: "escalation to human",
      status: "forwarded",
      agent_action: "forward",
      outbound_action: "forward",
      outbound_recipients: ["human@example.com"],
      outbound_subject: "Fwd: Re: Pricing question",
      outbound_body: null,
      outbound_body_redacted: true,
      outbound_message_id: null,
      outbound_reason: "sender requested human review"
    };
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [sampleMessages[0], followUp] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(screen.getByRole("heading", { name: "Pricing question" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /Re: Pricing question/i }).length).toBeGreaterThan(0);

    fireEvent.click(screen.getAllByRole("button", { name: /Re: Pricing question/i })[0]);
    expect(screen.getByRole("heading", { name: "Re: Pricing question" })).toBeInTheDocument();
    expect(screen.getByText("escalation to human")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Diagnostics"));
    expect(screen.getByText("<auto-42@example.com>")).toBeInTheDocument();
  });

  it("renders history status variants and raw invalid timestamps", async () => {
    const variantMessages = [
      {
        ...sampleMessages[0],
        uid: 50,
        subject: "Processing update",
        status: "processing",
        outbound_action: null,
        outbound_recipients: [],
        outbound_subject: null,
        outbound_body: null,
        outbound_body_redacted: false,
        outbound_reason: null,
        updated_at: "not-a-date",
        logs: []
      },
      {
        ...sampleMessages[0],
        uid: 51,
        subject: "Retry failed update",
        status: "retryable_failed"
      },
      {
        ...sampleMessages[0],
        uid: 52,
        subject: "Archived update",
        status: "archived"
      }
    ];
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: variantMessages });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(screen.getByRole("heading", { name: "Processing update" })).toBeInTheDocument();
    expect(screen.getAllByText("not-a-date").length).toBeGreaterThan(0);
    expect(screen.getByText("No outbound body recorded.")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Timeline"));
    expect(screen.getByText("No log entries recorded.")).toBeInTheDocument();
    expect(screen.getAllByText("processing").length).toBeGreaterThan(0);
    expect(screen.getAllByText("retryable_failed").length).toBeGreaterThan(0);
    expect(screen.getAllByText("archived").length).toBeGreaterThan(0);
  });

  it("keeps config visible when processed message history fails to load", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ error: "database unavailable" }, { status: 500 });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    expect(await screen.findByText("database unavailable")).toBeInTheDocument();
    expect(screen.getByText("MCP servers")).toBeInTheDocument();
  });
});
