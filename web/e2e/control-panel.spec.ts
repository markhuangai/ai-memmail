import { expect, test, type Page } from "@playwright/test";
import {
  sampleClassification,
  sampleConfig,
  sampleConversationDetail,
  sampleConversations,
  sampleMessages
} from "../src/fixtures";
import type { AppConfig, ProcessedEmail } from "../src/types";

test("control panel login and core workflows", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "chromium", "covered by the desktop project");

  const mock = process.env.E2E_LIVE === "1" ? null : await installMockApi(page);

  await page.goto("/");
  await loginIfNeeded(page);
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();

  if (process.env.E2E_LIVE === "1") {
    await runLiveSmoke(page);
    return;
  }

  await expect(page.getByRole("button", { name: "Save changes" })).toHaveCount(0);
  await page.getByRole("button", { name: "Mailboxes" }).click();
  await expect(page.getByLabel("Poll seconds")).toBeVisible();
  await page.getByLabel("Poll seconds").fill("75");
  const configSave = page.waitForResponse(
    (response) =>
      response.request().method() === "PUT" && response.url().endsWith("/api/config")
  );
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect((await configSave).ok()).toBeTruthy();
  expect(mock?.savedConfigs.at(-1)?.mailboxes[0].poll_interval_seconds).toBe(75);

  await page.getByLabel("Poll seconds").fill("76");
  await page.getByRole("button", { name: "Refresh data" }).click();
  await expect(page.getByRole("dialog", { name: "Unsaved config changes" })).toBeVisible();
  await page.getByRole("button", { name: "Keep editing" }).click();
  await expect(page.getByLabel("Poll seconds")).toHaveValue("76");
  await page.getByRole("button", { name: "Refresh data" }).click();
  const guardedSave = page.waitForResponse(
    (response) =>
      response.request().method() === "PUT" && response.url().endsWith("/api/config")
  );
  await page.getByRole("button", { name: "Save and continue" }).click();
  await expect((await guardedSave).ok()).toBeTruthy();
  expect(mock?.savedConfigs.at(-1)?.mailboxes[0].poll_interval_seconds).toBe(76);

  await page.getByRole("button", { name: "Safety" }).click();
  await page.getByLabel("Ban kind").selectOption("domain");
  await page.getByLabel("Ban value").fill("bad.example");
  await page.getByLabel("Ban reason").fill("jailbreak");
  await page.getByRole("button", { name: "Add" }).click();
  await expect(page.getByText("bad.example")).toBeVisible();
  await page.locator("tr", { hasText: "bad.example" }).getByRole("button", { name: "Remove" }).click();
  await page.getByRole("button", { name: "Remove sender" }).click();
  await expect(page.getByText("bad.example")).toHaveCount(0);

  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Open Safety prompt" }).click();
  await expect(page.getByLabel("Safety prompt content")).toHaveValue("Original safety prompt");
  await page.getByLabel("Safety prompt content").fill("Updated safety prompt");
  const promptSave = page.waitForResponse(
    (response) =>
      response.request().method() === "PUT" && response.url().includes("/api/prompt-file")
  );
  await page.getByRole("button", { name: "Save prompt" }).click();
  await expect((await promptSave).ok()).toBeTruthy();
  expect(mock?.savedPrompts.at(-1)).toEqual({ content: "Updated safety prompt" });
  await expect(page.getByText("Prompt saved.")).toBeVisible();
  await page.getByRole("button", { name: "Close prompt" }).click();

  await page.getByRole("button", { name: "Rules" }).click();
  await expect(page.getByText("Auto-decline marketing/vendor outreach")).toBeVisible();
  await expect(page.getByText("category:marketing_vendor")).toBeVisible();

  await page.getByRole("button", { name: "History" }).click();
  await expect(page.getByRole("heading", { name: "Pricing question" })).toBeVisible();
  await createHandoffFromDetail(page, "mark.personal@example.com");

  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByLabel("Control panel key")).toBeVisible();
});

test("mobile drawer navigation reaches key panels", async ({ page }, testInfo) => {
  test.skip(process.env.E2E_LIVE === "1", "mocked responsive test only");
  test.skip(testInfo.project.name !== "mobile-chromium", "covered by the mobile project");

  await installMockApi(page);
  await page.goto("/");
  await loginIfNeeded(page);

  await page.getByRole("button", { name: "Open navigation" }).click();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.locator(".sidebar")).toHaveClass("sidebar");
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByLabel("AI model")).toHaveValue("gpt-test");

  await page.getByRole("button", { name: "Open navigation" }).click();
  await page.getByRole("button", { name: "History" }).click();
  await expect(page.locator(".sidebar")).toHaveClass("sidebar");
  await expect(page.getByRole("heading", { name: "Pricing question" })).toBeVisible();
  await expect(page.getByLabel("Decision evidence")).toBeVisible();
});

async function runLiveSmoke(page: Page) {
  await page.getByRole("button", { name: "Mailboxes" }).click();
  if (!(await page.getByLabel("Poll seconds").isVisible().catch(() => false))) {
    await page.getByRole("button", { name: "Add mailbox" }).click();
  }
  await expect(page.getByLabel("Poll seconds")).toBeVisible();
  await page.getByRole("button", { name: "Safety" }).click();
  await expect(page.getByText("Banned senders")).toBeVisible();
  await page.getByRole("button", { name: "Rules" }).click();
  await expect(page.getByText("Auto-decline marketing/vendor outreach")).toBeVisible();
  await page.getByRole("button", { name: "History" }).click();

  const runId = process.env.AI_MEMMAIL_LIVE_E2E_RUN_ID;
  expect(runId, "AI_MEMMAIL_LIVE_E2E_RUN_ID should be exported for live e2e").toBeTruthy();
  for (const subject of [
    `live-e2e known mcp ${runId}`,
    `Re: live-e2e known mcp ${runId}`,
    `live-e2e human forward ${runId}`,
    `live-e2e quarantine ${runId}`,
    `live-e2e banned sender ${runId}`
  ]) {
    await expect(
      page.getByRole("button").filter({ hasText: subject }).first()
    ).toBeVisible();
  }
  await page
    .getByRole("button")
    .filter({ hasText: `live-e2e known mcp ${runId}` })
    .first()
    .click();
  await createHandoffFromDetail(page, await liveHandoffDestination(page));
}

async function installMockApi(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(globalThis.crypto, "randomUUID", {
      configurable: true,
      value: undefined
    });
  });

  let authenticated = false;
  let config = clone(sampleConfig);
  let messages: ProcessedEmail[] = clone(sampleMessages);
  const savedConfigs: AppConfig[] = [];
  const savedPrompts: Array<{ content: string }> = [];
  const handoff = {
    state: "active",
    destination: "mark.personal@example.com",
    remote_target: "person@example.com",
    last_error: null,
    updated_at: "2026-07-01 00:04:00+00"
  };

  await page.route("**/api/status", async (route) => {
    await route.fulfill({
      json: {
        service: "ai-memmail",
        authenticated,
        uptime_seconds: 5,
        enabled_mailboxes: config.mailboxes.filter((mailbox) => mailbox.enabled).length
      }
    });
  });
  await page.route("**/api/login", async (route) => {
    authenticated = true;
    await route.fulfill({
      headers: { "set-cookie": "ai_memmail_session=test; Path=/; SameSite=Strict" },
      json: { authenticated: true }
    });
  });
  await page.route("**/api/logout", async (route) => {
    authenticated = false;
    await route.fulfill({
      headers: { "set-cookie": "ai_memmail_session=; Max-Age=0; Path=/" },
      json: { authenticated: false }
    });
  });
  await page.route("**/api/config", async (route) => {
    if (route.request().method() === "PUT") {
      config = route.request().postDataJSON() as AppConfig;
      savedConfigs.push(clone(config));
    }
    await route.fulfill({ json: { config } });
  });
  await page.route("**/api/prompt-file**", async (route) => {
    if (route.request().method() === "PUT") {
      const body = route.request().postDataJSON() as { content: string };
      savedPrompts.push(body);
      await route.fulfill({ json: { path: "safety-scan.md", content: body.content } });
      return;
    }
    await route.fulfill({
      json: {
        path: "safety-scan.md",
        content: "Original safety prompt"
      }
    });
  });
  await page.route(/\/api\/messages\/[^/]+\/handoff$/, async (route) => {
    const body = route.request().postDataJSON() as {
      request_id?: string;
      destination?: string;
    };
    expect(body.request_id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
    );
    expect(body.destination).toBe(handoff.destination);
    messages = [{ ...sampleMessages[0], handoff }, ...sampleMessages.slice(1)];
    await route.fulfill({ json: { handoff } });
  });
  await page.route(/\/api\/conversations\/[^/]+\/messages$/, async (route) => {
    await route.fulfill({ json: { conversation: sampleConversationDetail } });
  });
  await page.route(/\/api\/conversations\/[^/]+$/, async (route) => {
    await route.fulfill({ json: { conversation: sampleConversationDetail } });
  });
  await page.route(/\/api\/conversations(?:\?[^/]*)?$/, async (route) => {
    await route.fulfill({ json: { conversations: sampleConversations } });
  });
  await page.route(/\/api\/messages(?:\?[^/]*)?$/, async (route) => {
    await route.fulfill({ json: { messages } });
  });
  await page.route("**/api/email-classification", async (route) => {
    await route.fulfill({ json: { classification: sampleClassification } });
  });

  return { savedConfigs, savedPrompts };
}

async function loginIfNeeded(page: Page) {
  if (await page.getByLabel("Control panel key").isVisible().catch(() => false)) {
    await page.getByLabel("Control panel key").fill(process.env.CONTROL_PANEL_KEY ?? "panel-key");
    await page.getByRole("button", { name: "Login" }).click();
  }
}

async function createHandoffFromDetail(page: Page, destination: string) {
  await page.getByRole("button", { name: /^hand off$/i }).click();
  await page.getByLabel(/handoff destination/i).fill(destination);
  const handoffResponse = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      response.url().includes("/api/messages/") &&
      response.url().endsWith("/handoff")
  );
  await page.getByRole("button", { name: /forward chain/i }).click();
  await expect((await handoffResponse).ok()).toBeTruthy();
  await expect(page.getByText(`Handed off to ${destination}`)).toBeVisible();
}

async function liveHandoffDestination(page: Page): Promise<string> {
  const payload = (await page.evaluate(async () => {
    const response = await fetch("/api/config");
    if (!response.ok) {
      throw new Error(`config request failed: ${response.status}`);
    }
    return response.json();
  })) as {
    config?: {
      mailboxes?: Array<{
        enabled?: boolean;
        safety_forward_to?: string[];
        agent?: { default_forward_to?: string[] };
      }>;
    };
  };
  const mailboxes = payload.config?.mailboxes ?? [];
  const mailbox = mailboxes.find((entry) => entry.enabled) ?? mailboxes[0];
  const forwardTo = mailbox?.agent?.default_forward_to?.[0] ?? mailbox?.safety_forward_to?.[0];
  expect(forwardTo, "live config should provide a forwarding mailbox").toBeTruthy();
  return handoffAlias(forwardTo ?? "");
}

function handoffAlias(address: string): string {
  const email = address.match(/<([^>]+)>/)?.[1] ?? address.trim();
  const at = email.lastIndexOf("@");
  expect(at, `forwarding address should be an email address: ${address}`).toBeGreaterThan(0);
  const local = email.slice(0, at).split("+")[0];
  const domain = email.slice(at + 1);
  return `${local}+handoff@${domain}`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}
