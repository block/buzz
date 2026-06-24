import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/leadership";

// Mock agent pubkeys (distinct from the relay agents seeded by default).
const AGENT_PAUL = "aa".repeat(32);
const AGENT_DUNCAN = "bb".repeat(32);

type LeadershipInstance = { instanceId: string; isLeader: boolean };

async function waitForBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_SEED_LEADERSHIP__?: unknown })
        .__BUZZ_E2E_SEED_LEADERSHIP__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForBridge(page);
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("unified-agents-groups")).toBeVisible({
    timeout: 10_000,
  });
}

// The freshest-leader rule selects max(lastSeen), tie-broken by seq. The seed
// hook seeds in array order with a monotonic seq, so list the intended leader
// LAST to make selection deterministic even when timestamps collide at ms.
async function seedLeadership(
  page: import("@playwright/test").Page,
  agentPubkey: string,
  instances: LeadershipInstance[],
) {
  await page.evaluate(
    ({ pubkey, frames }) => {
      const win = window as Window & {
        __BUZZ_E2E_SEED_LEADERSHIP__?: (input: {
          agentPubkey: string;
          instances: { instanceId: string; isLeader: boolean }[];
        }) => void;
      };
      win.__BUZZ_E2E_SEED_LEADERSHIP__?.({
        agentPubkey: pubkey,
        instances: frames,
      });
    },
    { pubkey: agentPubkey, frames: instances },
  );
}

const MANAGED_AGENTS = [
  {
    pubkey: AGENT_PAUL,
    name: "Paul",
    status: "running" as const,
    channelNames: ["general", "engineering"],
  },
  {
    pubkey: AGENT_DUNCAN,
    name: "Duncan",
    status: "running" as const,
    channelNames: ["general", "design"],
  },
];

async function openLeadershipSubmenu(
  page: import("@playwright/test").Page,
  agentPubkey: string,
) {
  await page.getByTestId(`managed-agent-actions-${agentPubkey}`).click();
  const submenuTrigger = page.getByRole("menuitem", { name: "Leadership" });
  await expect(submenuTrigger).toBeVisible();
  await submenuTrigger.hover();
  // Settle the submenu open animation before capture.
  await submenuTrigger.evaluate((el) =>
    Promise.all(
      el
        .closest("[data-state]")
        ?.getAnimations()
        .map((a) => a.finished) ?? [],
    ),
  );
}

test.describe("leadership UI screenshots", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("01 — single instance shows Leader badge, no submenu", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);

    await seedLeadership(page, AGENT_PAUL, [
      { instanceId: "4821-1718600000000000", isLeader: true },
    ]);

    const row = page.getByTestId(`managed-agent-${AGENT_PAUL}`);
    await expect(row).toContainText("Leader", { timeout: 5_000 });

    await page.getByTestId(`managed-agent-actions-${AGENT_PAUL}`).click();
    await expect(
      page.getByRole("menuitem", { name: "Leadership" }),
    ).toHaveCount(0);
    await page.keyboard.press("Escape");

    await page.getByTestId("unified-agents-groups").screenshot({
      path: `${SHOTS}/01-single-instance-leader.png`,
    });
  });

  test("02 — multi-instance badge reflects the freshest leader", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);

    // Leader seeded last → highest seq → wins the freshest-leader tie-break.
    await seedLeadership(page, AGENT_PAUL, [
      { instanceId: "4821-1718600000000000", isLeader: false },
      { instanceId: "5190-1718600100000000", isLeader: false },
      { instanceId: "6033-1718600200000000", isLeader: true },
    ]);

    const row = page.getByTestId(`managed-agent-${AGENT_PAUL}`);
    await expect(row).toContainText("Leader", { timeout: 5_000 });

    await page.getByTestId("unified-agents-groups").screenshot({
      path: `${SHOTS}/02-multi-instance-badge.png`,
    });
  });

  test("03 — leadership submenu lists each instance", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);

    await seedLeadership(page, AGENT_PAUL, [
      { instanceId: "4821-1718600000000000", isLeader: false },
      { instanceId: "5190-1718600100000000", isLeader: false },
      { instanceId: "6033-1718600200000000", isLeader: true },
    ]);

    const row = page.getByTestId(`managed-agent-${AGENT_PAUL}`);
    await expect(row).toContainText("Leader", { timeout: 5_000 });

    await openLeadershipSubmenu(page, AGENT_PAUL);
    await expect(page.getByRole("menuitem", { name: /Leader/ })).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/03-leadership-submenu.png`,
      clip: { x: 0, y: 0, width: 1280, height: 720 },
    });
  });

  test("04 — Make leader action on a non-leader instance", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);

    await seedLeadership(page, AGENT_PAUL, [
      { instanceId: "4821-1718600000000000", isLeader: false },
      { instanceId: "5190-1718600100000000", isLeader: false },
      { instanceId: "6033-1718600200000000", isLeader: true },
    ]);

    const row = page.getByTestId(`managed-agent-${AGENT_PAUL}`);
    await expect(row).toContainText("Leader", { timeout: 5_000 });

    await openLeadershipSubmenu(page, AGENT_PAUL);

    // A non-leader instance's item is the enabled cooperative-steal entry point.
    const makeLeaderItem = page
      .getByRole("menuitem")
      .filter({ hasText: "4821" });
    await expect(makeLeaderItem).toBeVisible();
    await makeLeaderItem.hover();

    await page.screenshot({
      path: `${SHOTS}/04-make-leader-action.png`,
      clip: { x: 0, y: 0, width: 1280, height: 720 },
    });
  });
});
