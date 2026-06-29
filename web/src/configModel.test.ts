import { describe, expect, it } from "vitest";
import {
  addBannedSender,
  addMailbox,
  addMcpServer,
  displaySecret,
  envToText,
  listToText,
  mailboxRouteLabel,
  removeBannedSender,
  removeMailbox,
  removeMcpServer,
  setListFromText,
  setMailboxScalar,
  summarizeConfig,
  textToEnv,
  updateMcpServer
} from "./configModel";
import { sampleConfig } from "./fixtures";

describe("configModel", () => {
  it("summarizes dashboard counts", () => {
    expect(summarizeConfig(sampleConfig)).toEqual({
      mailboxCount: 1,
      enabledMailboxes: 1,
      mcpServerCount: 1,
      bannedSenderCount: 1,
      averagePollSeconds: 60
    });
  });

  it("formats mailbox routing labels", () => {
    expect(mailboxRouteLabel(sampleConfig.mailboxes[0])).toBe("1 MCP / 1 reviewer");
  });

  it("updates mailbox scalar values immutably", () => {
    const next = setMailboxScalar(
      sampleConfig,
      "support",
      "poll_interval_seconds",
      90
    );
    expect(next.mailboxes[0].poll_interval_seconds).toBe(90);
    expect(sampleConfig.mailboxes[0].poll_interval_seconds).toBe(60);
  });

  it("adds and removes a default mailbox", () => {
    const empty = { ...sampleConfig, mailboxes: [] };
    const withMailbox = addMailbox(empty);
    expect(withMailbox.mailboxes[0]).toMatchObject({
      id: "mailbox_1",
      enabled: false,
      poll_interval_seconds: 60,
      safety_forward_to: ["review@example.com"],
      agent: { system_prompt_path: "support-agent.md" }
    });
    expect(removeMailbox(withMailbox, "mailbox_1").mailboxes).toHaveLength(0);
  });

  it("converts comma separated list fields", () => {
    expect(setListFromText("a@example.com, b@example.com, ")).toEqual([
      "a@example.com",
      "b@example.com"
    ]);
    expect(listToText(["a", "b"])).toBe("a, b");
  });

  it("converts MCP env text fields", () => {
    expect(textToEnv("A=1\ninvalid\nB = two")).toEqual({ A: "1", B: "two" });
    expect(envToText({ A: "1", B: "two" })).toBe("A=1\nB=two");
  });

  it("adds, updates, and removes MCP servers", () => {
    const withServer = addMcpServer(sampleConfig);
    expect(withServer.mcp_servers.dense_mem_2).toMatchObject({
      transport: "stdio",
      command: "npx"
    });

    const updated = updateMcpServer(withServer, "dense_mem_2", (server) => ({
      ...server,
      transport: "streamable_http",
      command: null,
      url: "http://dense-mem:8080/mcp"
    }));
    expect(updated.mcp_servers.dense_mem_2).toMatchObject({
      transport: "streamable_http",
      command: null,
      url: "http://dense-mem:8080/mcp"
    });

    const removed = removeMcpServer(updated, "dense_mem_primary");
    expect(removed.mcp_servers.dense_mem_primary).toBeUndefined();
    expect(removed.mailboxes[0].mcp_servers).toEqual([]);
  });

  it("upserts and removes banned senders", () => {
    const withSender = addBannedSender(sampleConfig, {
      kind: "email",
      value: "bad@example.com",
      reason: "prompt injection"
    });
    expect(withSender.banned_senders).toHaveLength(2);
    const removed = removeBannedSender(withSender, withSender.banned_senders[0]);
    expect(removed.banned_senders).toHaveLength(1);
    expect(removed.banned_senders[0].value).toBe("bad@example.com");
  });

  it("labels redacted and missing secrets", () => {
    expect(displaySecret("********")).toBe("configured");
    expect(displaySecret("real")).toBe("set");
    expect(displaySecret("")).toBe("missing");
  });
});
