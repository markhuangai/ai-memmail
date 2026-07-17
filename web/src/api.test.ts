import { describe, expect, it, vi } from "vitest";
import {
  ApiError,
  createHandoff,
  createEmailCategory,
  createEmailRule,
  createEmailTopic,
  deleteEmailRule,
  loadConfig,
  loadEmailClassification,
  loadMessages,
  loadPromptFile,
  loadStatus,
  login,
  saveConfig,
  savePromptFile,
  updateEmailRule
} from "./api";
import { sampleClassification, sampleConfig, sampleMessages } from "./fixtures";

function jsonResponse(body: unknown, init?: ResponseInit) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
    ...init
  });
}

describe("api", () => {
  it("loads status through same-origin request settings", async () => {
    const fetchImpl = vi.fn(async () =>
      jsonResponse({
        service: "ai-memmail",
        authenticated: true,
        uptime_seconds: 5,
        enabled_mailboxes: 1
      })
    );
    await expect(loadStatus(fetchImpl as typeof fetch)).resolves.toMatchObject({
      authenticated: true
    });
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/status",
      expect.objectContaining({ credentials: "same-origin" })
    );
  });

  it("posts login keys as JSON", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ authenticated: true }));
    await login("panel-key", fetchImpl as typeof fetch);
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/login",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ key: "panel-key" })
      })
    );
  });

  it("unwraps config payloads", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ config: sampleConfig }));
    await expect(loadConfig(fetchImpl as typeof fetch)).resolves.toEqual(sampleConfig);
  });

  it("unwraps processed message payloads", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ messages: sampleMessages }));
    await expect(loadMessages(fetchImpl as typeof fetch)).resolves.toEqual(sampleMessages);
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/messages",
      expect.objectContaining({ credentials: "same-origin" })
    );
  });

  it("defaults missing processed message payloads to an empty list", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({}));
    await expect(loadMessages(fetchImpl as typeof fetch)).resolves.toEqual([]);
  });

  it("loads processed messages with a limit", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ messages: sampleMessages }));
    await loadMessages(250, fetchImpl as typeof fetch);
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/messages?limit=250",
      expect.objectContaining({ credentials: "same-origin" })
    );
  });

  it("posts thread handoff requests with an idempotency key", async () => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue("11111111-1111-4111-8111-111111111111");
    const handoff = {
      state: "active",
      destination: "mark.personal@example.com",
      remote_target: "person@example.com",
      last_error: null,
      updated_at: "2026-07-01 00:04:00+00"
    };
    const fetchImpl = vi.fn(async () => jsonResponse({ handoff }));

    await expect(
      createHandoff(
        "2e7bcb41-5034-45a4-8135-3c33e6275d67",
        "mark.personal@example.com",
        fetchImpl as typeof fetch
      )
    ).resolves.toEqual(handoff);
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/messages/2e7bcb41-5034-45a4-8135-3c33e6275d67/handoff",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          request_id: "11111111-1111-4111-8111-111111111111",
          destination: "mark.personal@example.com"
        })
      })
    );
  });

  it("falls back to getRandomValues when randomUUID is unavailable", async () => {
    const randomUUIDDescriptor = Object.getOwnPropertyDescriptor(crypto, "randomUUID");
    Object.defineProperty(crypto, "randomUUID", {
      configurable: true,
      value: undefined
    });
    const getRandomValues = vi.spyOn(crypto, "getRandomValues").mockImplementation((array) => {
      const bytes = array as Uint8Array;
      bytes.set([
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        0x0d, 0x0e, 0x0f
      ]);
      return array;
    });
    let handoffRequest = "";
    const fetchImpl = vi.fn(async (_path: RequestInfo | URL, init?: RequestInit) => {
      handoffRequest = String(init?.body);
      return jsonResponse({ handoff: null });
    });

    try {
      await createHandoff(
        "2e7bcb41-5034-45a4-8135-3c33e6275d67",
        "mark.personal@example.com",
        fetchImpl as typeof fetch
      );
    } finally {
      getRandomValues.mockRestore();
      if (randomUUIDDescriptor) {
        Object.defineProperty(crypto, "randomUUID", randomUUIDDescriptor);
      } else {
        delete (crypto as Partial<Crypto>).randomUUID;
      }
    }

    expect(JSON.parse(handoffRequest)).toEqual({
      request_id: "00010203-0405-4607-8809-0a0b0c0d0e0f",
      destination: "mark.personal@example.com"
    });
  });

  it("loads and saves prompt files", async () => {
    const fetchImpl = vi.fn(async () =>
      jsonResponse({ path: "support-agent.md", content: "Prompt content" })
    );

    await expect(loadPromptFile("support-agent.md", fetchImpl as typeof fetch)).resolves.toEqual({
      path: "support-agent.md",
      content: "Prompt content"
    });
    await savePromptFile("support-agent.md", "Updated prompt", fetchImpl as typeof fetch);

    expect(fetchImpl).toHaveBeenNthCalledWith(
      1,
      "/api/prompt-file?path=support-agent.md",
      expect.objectContaining({ credentials: "same-origin" })
    );
    expect(fetchImpl).toHaveBeenNthCalledWith(
      2,
      "/api/prompt-file?path=support-agent.md",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ content: "Updated prompt" })
      })
    );
  });

  it("unwraps email classification payloads", async () => {
    const fetchImpl = vi.fn(async () =>
      jsonResponse({ classification: sampleClassification })
    );
    await expect(loadEmailClassification(fetchImpl as typeof fetch)).resolves.toEqual(
      sampleClassification
    );
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/email-classification",
      expect.objectContaining({ credentials: "same-origin" })
    );
  });

  it("creates email categories and topics", async () => {
    const fetchImpl = vi.fn(async () =>
      jsonResponse({ classification: sampleClassification })
    );

    await createEmailCategory("partner", "Partner outreach", fetchImpl as typeof fetch);
    await createEmailTopic("dense-mem", "Dense-Mem", fetchImpl as typeof fetch);

    expect(fetchImpl).toHaveBeenNthCalledWith(
      1,
      "/api/email-categories",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ name: "partner", description: "Partner outreach" })
      })
    );
    expect(fetchImpl).toHaveBeenNthCalledWith(
      2,
      "/api/email-topics",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ name: "dense-mem", description: "Dense-Mem" })
      })
    );
  });

  it("creates updates and deletes email rules", async () => {
    const fetchImpl = vi.fn(async () =>
      jsonResponse({ classification: sampleClassification })
    );
    const rule = {
      mailbox_id: "support",
      name: "Decline agency",
      category_id: 1,
      topic_ids: [3],
      action: "reply" as const,
      reply_goal: "Decline politely.",
      enabled: true,
      priority: 50
    };

    await createEmailRule(rule, fetchImpl as typeof fetch);
    await updateEmailRule(42, { ...rule, enabled: false }, fetchImpl as typeof fetch);
    await deleteEmailRule(42, fetchImpl as typeof fetch);

    expect(fetchImpl).toHaveBeenNthCalledWith(
      1,
      "/api/email-rules",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify(rule)
      })
    );
    expect(fetchImpl).toHaveBeenNthCalledWith(
      2,
      "/api/email-rules/42",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ ...rule, enabled: false })
      })
    );
    expect(fetchImpl).toHaveBeenNthCalledWith(
      3,
      "/api/email-rules/42",
      expect.objectContaining({ method: "DELETE" })
    );
  });

  it("saves config payloads", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ config: sampleConfig }));
    await saveConfig(sampleConfig, fetchImpl as typeof fetch);
    expect(fetchImpl).toHaveBeenCalledWith(
      "/api/config",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify(sampleConfig)
      })
    );
  });

  it("throws API errors with response status", async () => {
    const fetchImpl = vi.fn(async () =>
      jsonResponse({ error: "control panel login required" }, { status: 401 })
    );
    await expect(loadConfig(fetchImpl as typeof fetch)).rejects.toEqual(
      new ApiError("control panel login required", 401)
    );
  });

  it("uses a generic API error when the error payload is missing", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({}, { status: 502 }));
    await expect(loadConfig(fetchImpl as typeof fetch)).rejects.toEqual(
      new ApiError("request failed", 502)
    );
  });
});
