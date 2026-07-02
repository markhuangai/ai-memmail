import { expect, test } from "@playwright/test";
import { sampleConfig, sampleMessages } from "../src/fixtures";

test("control panel login and mailbox edit", async ({ page }) => {
  if (process.env.E2E_LIVE !== "1") {
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
    await page.route("**/api/messages", async (route) => {
      await route.fulfill({ json: { messages: sampleMessages } });
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
    return;
  }

  await expect(page.getByRole("heading", { name: "Pricing question" })).toBeVisible();
});

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
