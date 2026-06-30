import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const AGENT = "ce".repeat(32);
const SHOTS = "tests/e2e/__shots__";

async function openAndDebug(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("app-sidebar").waitFor({ state: "visible" });
  await page.getByTestId("channel-engineering").click();
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await page.waitForTimeout(400);
  await page.getByTestId("channel-members-trigger").first().click();
  await page.getByTestId("members-sidebar").waitFor({ state: "visible" });
  await page.waitForTimeout(300);
  await page.getByTestId(`sidebar-member-menu-${AGENT}`).click({ force: true });
  await page.getByTestId(`sidebar-view-activity-${AGENT}`).click();
  await page
    .getByTestId("agent-session-thread-panel")
    .waitFor({ state: "visible" });
  await page.waitForTimeout(500);
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  await page.getByTestId("agent-session-toggle-debug-render-classes").click();
  // Dismiss the dropdown with an outside-click (Escape was leaving the menu open
  // and overlapping the top of the expanded captures). Click the chat title area,
  // which is well clear of the Activity panel, then confirm the menu is gone.
  await page.getByTestId("chat-title").click({ force: true });
  await expect(
    page.getByTestId("agent-session-toggle-debug-render-classes"),
  ).toHaveCount(0);
  await page.waitForTimeout(500);
}

test("expanded disclosure rows", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT,
        name: "Cerberus",
        status: "running",
        channelNames: ["engineering"],
        backend: { type: "local" },
      },
    ],
  });
  await page.setViewportSize({ width: 1280, height: 1400 });
  await openAndDebug(page);
  const panel = page.getByTestId("agent-session-thread-panel");

  // Expand every collapsible disclosure row in the feed.
  // Tool rows use native <details>; thought/prompt rows use aria-expanded buttons.
  await panel.evaluate((root) => {
    root.querySelectorAll("details").forEach((d) => {
      (d as HTMLDetailsElement).open = true;
    });
  });
  const expanders = panel.locator(
    '[aria-expanded="false"]:not([data-testid="agent-session-settings-menu-trigger"])',
  );
  const n = await expanders.count();
  for (let i = 0; i < n; i++) {
    const el = expanders.nth(i);
    try {
      await el.click({ force: true, timeout: 1000 });
    } catch {}
    await page.waitForTimeout(80);
  }
  // Belt-and-suspenders: ensure the settings dropdown is not open before we
  // screenshot (the expander loop or a stray focus can reopen it). Click the
  // chat title (well clear of the Activity panel) and confirm the menu is gone.
  if (
    await page.getByTestId("agent-session-toggle-debug-render-classes").count()
  ) {
    await page.getByTestId("chat-title").click({ force: true });
    await expect(
      page.getByTestId("agent-session-toggle-debug-render-classes"),
    ).toHaveCount(0);
  }
  await page.waitForTimeout(500);

  const sc = panel.locator(".overflow-y-auto").first();
  const m = await sc.evaluate((el) => {
    (el as HTMLElement).scrollTop = 0;
    return { sh: el.scrollHeight, ch: el.clientHeight };
  });
  await page.waitForTimeout(300);
  const step = Math.max(200, m.ch - 140);
  let y = 0,
    i = 0;
  while (y < m.sh && i < 16) {
    await sc.evaluate((el, top) => {
      (el as HTMLElement).scrollTop = top;
    }, y);
    await page.waitForTimeout(300);
    await panel.screenshot({
      path: `${SHOTS}/expanded-${String(i).padStart(2, "0")}.png`,
    });
    y += step;
    i += 1;
  }
  console.log("EXPMETRICS:" + JSON.stringify({ ...m, expanders: n, steps: i }));
});
