import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleConfig, sampleMessages } from "./fixtures";
import type { AppConfig } from "./types";

function jsonResponse(body: unknown, init?: ResponseInit) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
      ...init
    })
  );
}

describe("App", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("logs in and renders overview metrics", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementationOnce(() =>
        jsonResponse({
          service: "ai-memmail",
          authenticated: false,
          uptime_seconds: 0,
          enabled_mailboxes: 0
        })
      )
      .mockImplementationOnce(() => jsonResponse({ authenticated: true }))
      .mockImplementationOnce(() =>
        jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        })
      )
      .mockImplementationOnce(() => jsonResponse({ config: sampleConfig }))
      .mockImplementationOnce(() => jsonResponse({ messages: sampleMessages }));

    render(<App />);

    fireEvent.change(await screen.findByLabelText(/control panel key/i), {
      target: { value: "panel-key" }
    });
    fireEvent.click(screen.getByRole("button", { name: /login/i }));

    expect(await screen.findByText("MCP servers")).toBeInTheDocument();
    expect(screen.getByText("1/1")).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(5);
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
      if (path === "/api/messages") {
        return jsonResponse({ messages: sampleMessages });
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
    expect(screen.getAllByText("prompt_injection").length).toBeGreaterThan(0);
    expect(screen.getByText("Redacted prompt-injection sample requesting instruction override and secret disclosure.")).toBeInTheDocument();
    expect(screen.getByText(/Forward body omitted/i)).toBeInTheDocument();
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
      if (path === "/api/messages") {
        return jsonResponse({ messages: [] });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(screen.getByRole("heading", { name: "No processed messages" })).toBeInTheDocument();
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
      if (path === "/api/messages") {
        return jsonResponse({ messages: [sampleMessages[0], followUp] });
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
      if (path === "/api/messages") {
        return jsonResponse({ messages: variantMessages });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
    expect(screen.getByRole("heading", { name: "Processing update" })).toBeInTheDocument();
    expect(screen.getAllByText("not-a-date").length).toBeGreaterThan(0);
    expect(screen.getByText("No outbound body recorded.")).toBeInTheDocument();
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
      if (path === "/api/messages") {
        return jsonResponse({ error: "database unavailable" }, { status: 500 });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    expect(await screen.findByText("database unavailable")).toBeInTheDocument();
    expect(screen.getByText("MCP servers")).toBeInTheDocument();
  });

  it("edits mailbox polling and saves config", async () => {
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        return jsonResponse({ config: sampleConfig });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mailboxes/i }));
    fireEvent.change(screen.getByLabelText(/poll seconds/i), {
      target: { value: "75" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    expect(JSON.parse(savedBodies[0]).mailboxes[0].poll_interval_seconds).toBe(75);
  });

  it("edits mailbox routing and transport fields", async () => {
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        return jsonResponse({ config: JSON.parse(String(init.body)) as AppConfig });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mailboxes/i }));
    fireEvent.click(screen.getByLabelText(/enabled/i));
    fireEvent.change(screen.getByLabelText(/^address$/i), {
      target: { value: "ops@example.com" }
    });
    fireEvent.change(screen.getByLabelText(/safety forward/i), {
      target: { value: "review@example.com, lead@example.com" }
    });
    fireEvent.change(screen.getByLabelText(/mcp servers/i), {
      target: { value: "dense_mem_primary" }
    });
    fireEvent.change(screen.getByLabelText(/imap host/i), {
      target: { value: "imap.changed.test" }
    });
    fireEvent.change(screen.getByLabelText(/imap port/i), {
      target: { value: "1993" }
    });
    fireEvent.change(screen.getByLabelText(/imap user/i), {
      target: { value: "imap-user" }
    });
    fireEvent.change(screen.getByLabelText(/imap password/i), {
      target: { value: "imap-password" }
    });
    fireEvent.change(screen.getByLabelText(/imap folder/i), {
      target: { value: "Support" }
    });
    fireEvent.click(screen.getByLabelText(/imap tls/i));
    fireEvent.change(screen.getByLabelText(/smtp host/i), {
      target: { value: "smtp.changed.test" }
    });
    fireEvent.change(screen.getByLabelText(/smtp port/i), {
      target: { value: "2525" }
    });
    fireEvent.change(screen.getByLabelText(/smtp user/i), {
      target: { value: "smtp-user" }
    });
    fireEvent.change(screen.getByLabelText(/smtp password/i), {
      target: { value: "smtp-password" }
    });
    fireEvent.change(screen.getByLabelText(/smtp from/i), {
      target: { value: "ops@example.com" }
    });
    fireEvent.click(screen.getByLabelText(/smtp starttls/i));
    fireEvent.change(screen.getByLabelText(/agent prompt/i), {
      target: { value: "ops-agent.md" }
    });
    fireEvent.change(screen.getByLabelText(/default forward/i), {
      target: { value: "lead@example.com" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    const saved = JSON.parse(savedBodies[0]) as AppConfig;
    expect(saved.mailboxes[0]).toMatchObject({
      enabled: false,
      address: "ops@example.com",
      safety_forward_to: ["review@example.com", "lead@example.com"],
      imap: {
        host: "imap.changed.test",
        port: 1993,
        username: "imap-user",
        password: "imap-password",
        folder: "Support",
        tls: false
      },
      smtp: {
        host: "smtp.changed.test",
        port: 2525,
        username: "smtp-user",
        password: "smtp-password",
        from: "ops@example.com",
        starttls: false
      },
      agent: {
        system_prompt_path: "ops-agent.md",
        default_forward_to: ["lead@example.com"]
      }
    });
  });

  it("adds a mailbox from an empty config and saves it", async () => {
    const emptyConfig: AppConfig = {
      ...sampleConfig,
      mcp_servers: {},
      mailboxes: [],
      banned_senders: []
    };
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 0
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        return jsonResponse({ config: JSON.parse(String(init.body)) as AppConfig });
      }
      return jsonResponse({ config: emptyConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mailboxes/i }));
    expect(screen.getByText("No mailboxes configured")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /add mailbox/i }));
    expect(screen.getByText("mailbox_1@example.com")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/poll seconds/i), {
      target: { value: "90" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    const saved = JSON.parse(savedBodies[0]) as AppConfig;
    expect(saved.mailboxes[0]).toMatchObject({
      id: "mailbox_1",
      enabled: false,
      poll_interval_seconds: 90,
      safety_forward_to: ["review@example.com"],
      mcp_servers: []
    });
  });

  it("adds and edits MCP servers", async () => {
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        return jsonResponse({ config: JSON.parse(String(init.body)) as AppConfig });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mcp servers/i }));
    fireEvent.click(screen.getByRole("button", { name: /add server/i }));
    expect(screen.getByText("dense_mem_2")).toBeInTheDocument();
    fireEvent.change(screen.getAllByLabelText(/transport/i)[1], {
      target: { value: "streamable_http" }
    });
    fireEvent.change(screen.getAllByLabelText(/command/i)[1], {
      target: { value: "" }
    });
    fireEvent.change(screen.getAllByLabelText(/^url$/i)[1], {
      target: { value: "http://dense-mem:8080/mcp" }
    });
    fireEvent.change(screen.getAllByLabelText(/^env$/i)[1], {
      target: { value: "DENSE_MEM_API_KEY=local" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    const saved = JSON.parse(savedBodies[0]) as AppConfig;
    expect(saved.mcp_servers.dense_mem_2).toMatchObject({
      transport: "streamable_http",
      command: null,
      url: "http://dense-mem:8080/mcp",
      env: { DENSE_MEM_API_KEY: "local" }
    });
  });

  it("edits database, AI, prompt, and logging settings", async () => {
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        return jsonResponse({ config: JSON.parse(String(init.body)) as AppConfig });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^settings$/i }));
    fireEvent.change(screen.getByLabelText(/postgres host/i), {
      target: { value: "postgres.changed.test" }
    });
    fireEvent.change(screen.getByLabelText(/postgres port/i), {
      target: { value: "15432" }
    });
    fireEvent.change(screen.getByLabelText(/postgres user/i), {
      target: { value: "changed_user" }
    });
    fireEvent.change(screen.getByLabelText(/postgres password/i), {
      target: { value: "changed-password" }
    });
    fireEvent.change(screen.getByLabelText(/postgres database/i), {
      target: { value: "changed_db" }
    });
    fireEvent.change(screen.getByLabelText(/ai api url/i), {
      target: { value: "https://ai.changed.test/v1" }
    });
    fireEvent.change(screen.getByLabelText(/ai model/i), {
      target: { value: "model-changed" }
    });
    fireEvent.change(screen.getByLabelText(/prompt root/i), {
      target: { value: "./prompt-changed" }
    });
    fireEvent.change(screen.getByLabelText(/safety prompt/i), {
      target: { value: "scan-changed.md" }
    });
    fireEvent.change(screen.getByLabelText(/log level/i), {
      target: { value: "debug" }
    });
    fireEvent.change(screen.getByLabelText(/retention days/i), {
      target: { value: "30" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    const saved = JSON.parse(savedBodies[0]) as AppConfig;
    expect(saved.database).toMatchObject({
      host: "postgres.changed.test",
      port: 15432,
      username: "changed_user",
      password: "changed-password",
      database: "changed_db"
    });
    expect(saved.ai.AI_API_URL).toBe("https://ai.changed.test/v1");
    expect(saved.ai.AI_MODEL).toBe("model-changed");
    expect(saved.prompts).toMatchObject({
      root: "./prompt-changed",
      safety_scan: "scan-changed.md"
    });
    expect(saved.logging).toMatchObject({
      level: "debug",
      retention_days: 30
    });
  });

  it("adds and removes banned senders", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^safety$/i }));
    fireEvent.click(screen.getByRole("button", { name: /add/i }));
    expect(screen.queryByText("bad@example.com")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/ban kind/i), {
      target: { value: "domain" }
    });
    fireEvent.change(screen.getByLabelText(/ban value/i), {
      target: { value: "bad.example" }
    });
    fireEvent.change(screen.getByLabelText(/ban reason/i), {
      target: { value: "jailbreak" }
    });
    fireEvent.click(screen.getByRole("button", { name: /add/i }));

    expect(screen.getByText("bad.example")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: /remove/i })[0]);
    expect(screen.queryByText("blocked.example")).not.toBeInTheDocument();
  });

  it("renders MCP and settings tabs", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mcp servers/i }));
    expect(screen.getByText("dense_mem_primary")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^settings$/i }));
    expect(screen.getByLabelText(/ai model/i)).toHaveValue("gpt-test");
    expect(screen.getByLabelText(/postgres host/i)).toHaveValue("postgres");
  });

  it("shows save errors and logs out", async () => {
    let authenticated = true;
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/logout") {
        authenticated = false;
        return jsonResponse({ authenticated: false });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        return jsonResponse({ error: "invalid config" }, { status: 400 });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    await screen.findByText("Runtime");
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("invalid config");

    fireEvent.click(screen.getByRole("button", { name: /logout/i }));
    expect(await screen.findByLabelText(/control panel key/i)).toBeInTheDocument();
  });
});
