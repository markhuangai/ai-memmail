import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleConfig } from "./fixtures";
import { classificationResponse, jsonResponse } from "./testHelpers";
import type { AppConfig } from "./types";

describe("App config", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
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
      if (path === "/api/email-classification") {
        return classificationResponse();
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
      if (path === "/api/email-classification") {
        return classificationResponse();
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
    expect(screen.getByLabelText(/mcp server dense_mem_primary/i)).toBeChecked();
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
      mcp_servers: ["dense_mem_primary"],
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
      if (path === "/api/email-classification") {
        return classificationResponse();
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
      if (path === "/api/email-classification") {
        return classificationResponse();
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
    fireEvent.click(screen.getAllByRole("button", { name: /add variable/i })[1]);
    const variableInputs = screen.getAllByLabelText(/^variable$/i);
    fireEvent.change(variableInputs[variableInputs.length - 1], {
      target: { value: "DENSE_MEM_API_KEY" }
    });
    const valueInputs = screen.getAllByLabelText(/^value$/i);
    fireEvent.change(valueInputs[valueInputs.length - 1], {
      target: { value: "local" }
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

  it("removes MCP env rows and servers", async () => {
    const config: AppConfig = {
      ...sampleConfig,
      mcp_servers: {
        dense_mem_primary: {
          ...sampleConfig.mcp_servers.dense_mem_primary,
          env: {
            ENV_VAR_1: "existing",
            DENSE_MEM_API_KEY: "secret"
          }
        }
      }
    };
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
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mcp servers/i }));
    expect(screen.getByDisplayValue("secret")).toHaveAttribute("type", "password");
    fireEvent.click(screen.getByRole("button", { name: /add variable/i }));
    expect(screen.getByDisplayValue("ENV_VAR_3")).toBeInTheDocument();
    fireEvent.change(screen.getByDisplayValue("ENV_VAR_3"), {
      target: { value: "" }
    });
    expect(screen.queryByDisplayValue("ENV_VAR_3")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /remove dense_mem_api_key/i }));
    expect(screen.queryByDisplayValue("DENSE_MEM_API_KEY")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    expect(JSON.parse(savedBodies[0]).mcp_servers).toEqual({});
  });

  it("shows empty MCP assignment state for mailboxes", async () => {
    const config: AppConfig = {
      ...sampleConfig,
      mcp_servers: {},
      mailboxes: [
        {
          ...sampleConfig.mailboxes[0],
          mcp_servers: []
        }
      ]
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
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mailboxes/i }));
    expect(screen.getByText("No MCP servers configured.")).toBeInTheDocument();
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
      if (path === "/api/email-classification") {
        return classificationResponse();
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
    fireEvent.change(screen.getByLabelText(/safety prompt/i, { selector: "input" }), {
      target: { value: "scan-changed.md" }
    });
    fireEvent.change(screen.getByLabelText(/classifier prompt/i, { selector: "input" }), {
      target: { value: "classifier-changed.md" }
    });
    fireEvent.change(screen.getByLabelText(/rule action prompt/i, { selector: "input" }), {
      target: { value: "rule-changed.md" }
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
      safety_scan: "scan-changed.md",
      email_classifier: "classifier-changed.md",
      rule_action: "rule-changed.md"
    });
    expect(saved.logging).toMatchObject({
      level: "debug",
      retention_days: 30
    });
  });

  it("opens and saves individual prompt files from settings", async () => {
    const savedPromptBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/prompt-file") && init?.method === "PUT") {
        savedPromptBodies.push(String(init.body));
        return jsonResponse({
          path: "safety-scan.md",
          content: JSON.parse(String(init.body)).content as string
        });
      }
      if (String(path).startsWith("/api/prompt-file")) {
        return jsonResponse({
          path: "safety-scan.md",
          content: "Original safety prompt"
        });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^settings$/i }));
    fireEvent.click(screen.getByRole("button", { name: /open safety prompt/i }));
    const editor = await screen.findByLabelText(/safety prompt content/i);
    expect(editor).toHaveValue("Original safety prompt");
    fireEvent.change(editor, {
      target: { value: "Updated safety prompt" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save prompt/i }));

    await waitFor(() => expect(savedPromptBodies).toHaveLength(1));
    expect(JSON.parse(savedPromptBodies[0])).toEqual({
      content: "Updated safety prompt"
    });
    expect(await screen.findByText("Prompt saved.")).toBeInTheDocument();
  });
});
