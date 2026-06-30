import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const AGENT = "ce".repeat(32);
const SHOTS = "tests/e2e/__shots__";

async function openActivityPanel(page: import("@playwright/test").Page) {
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
  await page.waitForTimeout(600);
}

async function enableDebug(page: import("@playwright/test").Page) {
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  await page.getByTestId("agent-session-toggle-debug-render-classes").click();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);
}

function scroller(page: import("@playwright/test").Page) {
  return page
    .locator('[data-testid="agent-session-thread-panel"] .overflow-y-auto')
    .first();
}

test("debug fixture taxonomy", async ({ page }) => {
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
  // Tall viewport so each scroll step grabs a big slice of the feed.
  await page.setViewportSize({ width: 1280, height: 1400 });
  await openActivityPanel(page);
  await enableDebug(page);

  const panel = page.getByTestId("agent-session-thread-panel");
  const sc = scroller(page);

  // Scroll to top, then walk down capturing the panel each step.
  const metrics = await sc.evaluate((el) => {
    (el as HTMLElement).scrollTop = 0;
    return { sh: el.scrollHeight, ch: el.clientHeight };
  });
  await page.waitForTimeout(300);

  const step = Math.max(200, metrics.ch - 140); // overlap so nothing is lost at seams
  let y = 0;
  let i = 0;
  while (y < metrics.sh) {
    await sc.evaluate((el, top) => {
      (el as HTMLElement).scrollTop = top;
    }, y);
    await page.waitForTimeout(350);
    await panel.screenshot({
      path: `${SHOTS}/feed-${String(i).padStart(2, "0")}.png`,
    });
    y += step;
    i += 1;
    if (i > 12) break;
  }
  console.log("METRICS:" + JSON.stringify({ ...metrics, steps: i }));
});

test("cog menu open", async ({ page }) => {
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
  await page.setViewportSize({ width: 1280, height: 900 });
  await openActivityPanel(page);
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${SHOTS}/cog-menu.png` });
});

test("raw rail on", async ({ page }) => {
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
  await openActivityPanel(page);
  await enableDebug(page);
  // Toggle raw feed on too
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  await page.getByTestId("agent-session-toggle-raw-feed").click();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(600);
  const sc = scroller(page);
  await sc.evaluate((el) => {
    (el as HTMLElement).scrollTop = 0;
  });
  await page.waitForTimeout(300);
  await page
    .getByTestId("agent-session-thread-panel")
    .screenshot({ path: `${SHOTS}/raw-rail.png` });
});
