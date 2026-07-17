import { expect, test, type Page } from "@playwright/test";
import { sampleClassification, sampleConfig, sampleMessages } from "../src/fixtures";

test("control panel login and mailbox edit", async ({ page }) => {
  if (process.env.E2E_LIVE !== "1") {
    await page.addInitScript(() => {
      Object.defineProperty(globalThis.crypto, "randomUUID", {
        configurable: true,
        value: undefined
      });
    });
    const handoff = {
      state: "active",
      destination: "mark.personal@example.com",
      remote_target: "person@example.com",
      last_error: null,
      updated_at: "2026-07-01 00:04:00+00"
    };
    let messages = sampleMessages;
    await page.route("**/api/status", async (route) => {
      await route.fulfill({
        json: {
          service: "ai-memmail",
          authenticated: route.request().headers().cookie?.includes("ai_memmail_session") ?? false,
          uptime_seconds: 5,
          enabled_mailboxes: 1
        }
      });
    });
    await page.route("**/api/login", async (route) => {
      await route.fulfill({
        headers: { "set-cookie": "ai_memmail_session=test; Path=/; SameSite=Strict" },
        json: { authenticated: true }
      });
    });
    await page.route("**/api/config", async (route) => {
      await route.fulfill({ json: { config: sampleConfig } });
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
    await page.route(/\/api\/messages(?:\?[^/]*)?$/, async (route) => {
      await route.fulfill({ json: { messages } });
    });
    await page.route("**/api/email-classification", async (route) => {
      await route.fulfill({ json: { classification: sampleClassification } });
    });
  }

  await page.goto("/");
  if (await page.getByLabel("Control panel key").isVisible().catch(() => false)) {
    await page.getByLabel("Control panel key").fill(process.env.CONTROL_PANEL_KEY ?? "panel-key");
    await page.getByRole("button", { name: "Login" }).click();
  }

  await expect(page.getByRole("button", { name: "Mailboxes" })).toBeVisible();
  await page.getByRole("button", { name: "Mailboxes" }).click();
  if (!(await page.getByLabel("Poll seconds").isVisible().catch(() => false))) {
    await page.getByRole("button", { name: "Add mailbox" }).click();
  }
  await expect(page.getByLabel("Poll seconds")).toBeVisible();
  await page.getByLabel("Poll seconds").fill("75");
  await page.getByRole("button", { name: "Safety" }).click();
  await expect(page.getByText("Banned Senders")).toBeVisible();
  await page.getByRole("button", { name: "Rules" }).click();
  await expect(page.getByText("Auto-decline marketing/vendor outreach")).toBeVisible();
  await expect(page.getByText("category:marketing_vendor")).toBeVisible();
  await page.getByRole("button", { name: "History" }).click();
  if (process.env.E2E_LIVE === "1") {
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
    return;
  }

  await expect(page.getByRole("heading", { name: "Pricing question" })).toBeVisible();
  await createHandoffFromDetail(page, "mark.personal@example.com");
});

async function createHandoffFromDetail(page: Page, destination: string) {
  await page.getByRole("button", { name: /hand off thread/i }).click();
  await page.getByLabel(/handoff destination/i).fill(destination);
  const handoffResponse = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      response.url().includes("/api/messages/") &&
      response.url().endsWith("/handoff")
  );
  await page.getByRole("button", { name: /forward chain/i }).click();
  await expect((await handoffResponse).ok()).toBeTruthy();
  await expect(page.getByText("Handed off").first()).toBeVisible();
  await expect(page.getByText(new RegExp(`${escapeRegExp(destination)} to `))).toBeVisible();
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
