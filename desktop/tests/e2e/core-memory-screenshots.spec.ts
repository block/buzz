import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/core-memory";

const OBSERVER_AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const NOW = new Date("2025-06-15T12:00:00Z").toISOString();

const MANAGED_AGENTS = [
  {
    pubkey: OBSERVER_AGENT_PUBKEY,
    name: "Observer Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
];

const SYSTEM_PROMPT_WITH_CORE =
  "[Base]\nYou are a helpful AI assistant running in Buzz.\n\n" +
  "[System]\nYou are Observer Agent. You coordinate multi-agent workflows.\n\n" +
  "[Agent Memory — core]\n" +
  "I am Duncan — full-stack executor on the Buzz team.\n\n" +
  "## Lessons Learned\n\n" +
  "### Tag teammates on handoff — ALWAYS (CRITICAL)\n" +
  "After completing a task, @mention the next person in the workflow.\n\n" +
  "### Source claims require a fresh fetch (CRITICAL)\n" +
  "Never quote repo source off a stale clone.";

async function waitForSeedHook(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function openObserverFeedPanel(
  page: import("@playwright/test").Page,
  agentPubkey: string,
) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForSeedHook(page);
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();
  const profilePanel = page.getByTestId("user-profile-panel");
  await expect(profilePanel).toBeVisible({ timeout: 10_000 });
  const activityBtn = page.getByTestId(
    `user-profile-view-activity-${agentPubkey}`,
  );
  await expect(activityBtn).toBeVisible({ timeout: 5_000 });
  await activityBtn.click();
  const feedPanel = page.getByTestId("agent-session-thread-panel");
  await expect(feedPanel).toBeVisible({ timeout: 10_000 });
  return feedPanel;
}

async function seedObserverEvents(
  page: import("@playwright/test").Page,
  agentPubkey: string,
  events: Array<{
    seq: number;
    timestamp: string;
    kind: string;
    agentIndex: number | null;
    channelId: string | null;
    sessionId: string | null;
    turnId: string | null;
    payload: unknown;
  }>,
) {
  await page.evaluate(
    ({ pubkey, evts }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: evts,
      });
    },
    { pubkey: agentPubkey, evts: events },
  );
  await page.waitForTimeout(300);
}

async function settleAnimations(panel: import("@playwright/test").Locator) {
  await panel.evaluate((el) =>
    Promise.all(
      el
        .getAnimations({ subtree: true })
        .filter((a) => {
          const timing = a.effect?.getTiming();
          return timing?.iterations !== Number.POSITIVE_INFINITY;
        })
        .map((a) => a.finished),
    ),
  );
}

test.describe("core memory section screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error("PAGE ERROR:", err.message);
    });
  });

  test("01-core-memory-collapsed", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: null,
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/new",
          params: { systemPrompt: SYSTEM_PROMPT_WITH_CORE },
        },
      },
    ]);

    await expect(feedPanel.getByText("System prompt")).toBeVisible({
      timeout: 5_000,
    });

    // Open the outer details element so the sections are visible.
    await feedPanel.getByTestId("transcript-metadata-item").evaluate((el) => {
      if (el.tagName === "DETAILS") (el as HTMLDetailsElement).open = true;
      for (const details of el.querySelectorAll("details")) {
        details.open = true;
      }
    });

    // Wait for Core Memory section label to appear (collapsed by default).
    await expect(feedPanel.getByText("Core Memory")).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(feedPanel);

    // Crop to the transcript-metadata-item to show the sections tightly.
    const metadataItem = feedPanel.getByTestId("transcript-metadata-item");
    await metadataItem.screenshot({
      path: `${SHOTS}/01-core-memory-collapsed.png`,
    });
  });

  test("02-core-memory-expanded", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: null,
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/new",
          params: { systemPrompt: SYSTEM_PROMPT_WITH_CORE },
        },
      },
    ]);

    await expect(feedPanel.getByText("System prompt")).toBeVisible({
      timeout: 5_000,
    });

    await feedPanel.getByTestId("transcript-metadata-item").evaluate((el) => {
      if (el.tagName === "DETAILS") (el as HTMLDetailsElement).open = true;
      for (const details of el.querySelectorAll("details")) {
        details.open = true;
      }
    });

    // Expand all section accordions including Core Memory.
    const sectionButtons = feedPanel
      .getByTestId("transcript-metadata-item")
      .getByTestId("transcript-prompt-context-sections")
      .getByRole("button");
    const allButtons = await sectionButtons.all();
    expect(allButtons.length).toBeGreaterThan(0);
    for (const btn of allButtons) {
      await btn.click();
    }

    await expect(feedPanel.getByText("Core Memory")).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(feedPanel);

    const metadataItem = feedPanel.getByTestId("transcript-metadata-item");
    await metadataItem.screenshot({
      path: `${SHOTS}/02-core-memory-expanded.png`,
    });
  });
});
