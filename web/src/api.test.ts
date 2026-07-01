import { describe, expect, it, vi } from "vitest";
import { ApiError, loadConfig, loadMessages, loadStatus, login, saveConfig } from "./api";
import { sampleConfig, sampleMessages } from "./fixtures";

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
});
