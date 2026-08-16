import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// Screenshot spec for the "full" thread view mode (thread fills the main
// panel, channel sidebar stays on the left), rendered with the Honey identity.
test("full thread mode screenshot", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.channels.threadViewMode", "full");
    localStorage.setItem("buzz-identity-variant", "honey");
  });
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        name: "alice",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");

  const rootId = await page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content:
        "Can we get the supplier reconciliation report ready for Thursday's ops review?",
      createdAt: 1_700_900_000,
    });
    if (!root) throw new Error("Failed to seed thread root");
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content:
        "Yes — I will pull the deltas from last week and flag anything over 5%.",
      parentEventId: root.id,
      createdAt: 1_700_900_060,
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content:
        "Draft is up: three suppliers account for 80% of the variance. I annotated the outliers so you can skim it in five minutes.",
      parentEventId: root.id,
      createdAt: 1_700_900_120,
      pubkey:
        "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Perfect, that is exactly the shape I needed. Thanks both.",
      parentEventId: root.id,
      createdAt: 1_700_900_180,
    });
    return root.id;
  });

  await page.getByTestId("channel-general").click();
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();

  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/screenshots/thread-full-panel.png",
  });
});
