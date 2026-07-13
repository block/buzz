import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/activity-scope-label";

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents

// Open the activity pane via profile → "View activity" (same ingress the
// observer-feed screenshot spec uses).
async function openActivityFromChannel(
  page: import("@playwright/test").Page,
  channelTestId: string,
  channelTitle: string,
) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId(channelTestId).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelTitle);

  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();

  const profilePanel = page.getByTestId("user-profile-panel");
  await expect(profilePanel).toBeVisible({ timeout: 10_000 });

  const activityBtn = page.getByTestId(
    `user-profile-view-activity-${AGENT_PUBKEY}`,
  );
  await expect(activityBtn).toBeVisible({ timeout: 5_000 });
  await activityBtn.click();

  const panel = page.getByTestId("agent-session-thread-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  return panel;
}

test.describe("activity panel scope label", () => {
  test("channel-targeted pane shows the channel name", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Observer Agent",
          status: "running" as const,
          channelNames: ["agents"],
        },
      ],
    });

    const panel = await openActivityFromChannel(
      page,
      "channel-agents",
      "agents",
    );
    await expect(page.getByTestId("agent-session-scope-label")).toHaveText(
      "#agents",
    );

    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/01-channel-scoped.png` });
  });

  test("unscoped pane shows All channels", async ({ page }) => {
    // The agent lives in #random only. Restoring an agentSession URL on
    // #agents (where the agent is not in the activity list) puts the pane in
    // all-channels scope — the state that looked silently broken before the
    // scope label existed. The app uses a hash router, so the deep link goes
    // in the hash.
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Observer Agent",
          status: "running" as const,
          channelNames: ["random"],
        },
      ],
    });

    await page.goto(
      `/#/channels/${AGENTS_CHANNEL_ID}?agentSession=${AGENT_PUBKEY}`,
      { waitUntil: "domcontentloaded" },
    );
    await expect(page.getByTestId("chat-title")).toHaveText("agents");

    const panel = page.getByTestId("agent-session-thread-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId("agent-session-scope-label")).toHaveText(
      "All channels",
    );

    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/02-all-channels.png` });
  });
});
