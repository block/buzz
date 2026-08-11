import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

/**
 * The Attention preview feature ships default-off. Seed the localStorage
 * override before the app boots so the flagged UI renders under test.
 */
async function enableAttentionFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ attention: true }),
    );
  });
}

const SHOTS = "test-results/catchup";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";

type MockWindow = Window & {
  __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
    category: "mention" | "needs_action" | "activity" | "agent_activity";
    channel_id: string | null;
    channel_name: string;
    content: string;
    created_at: number;
    id: string;
    kind: number;
    pubkey: string;
    tags: string[][];
  }) => unknown;
};

test.describe("inbox catch up mode", () => {
  test("catch up reads unread summaries, dedups asks, and marks all read", async ({
    page,
  }) => {
    await enableAttentionFeature(page);
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });

    // Seed an unread activity message that leads with a TL;DR line.
    await page.waitForFunction(
      () =>
        typeof (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
        "function",
    );
    await page.evaluate(
      ({ bobPubkey, channelId }) => {
        (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?.({
          category: "activity",
          channel_id: channelId,
          channel_name: "general",
          content:
            "**TL;DR** Relay migration is complete and green.\n\nLong-form detail about the cutover, dashboards, and follow-up work goes on for several paragraphs here.",
          created_at: Math.floor(Date.now() / 1_000) - 120,
          id: "mock-feed-catchup-tldr",
          kind: 9,
          pubkey: bobPubkey,
          tags: [],
        });
      },
      { bobPubkey: BOB_PUBKEY, channelId: GENERAL_CHANNEL_ID },
    );

    await expect(page.getByTestId("home-inbox-list")).toBeVisible();

    // Unread badge before opening Catch up — opening must not change it.
    const homeBadge = page.getByTestId("sidebar-home-count");
    const badgeBefore = (await homeBadge.count())
      ? await homeBadge.textContent()
      : null;

    // Toggle in: the conversation list is replaced by the reading.
    await page.getByTestId("inbox-catchup-toggle").click();
    await expect(page.getByTestId("catchup-view")).toBeVisible();
    await expect(page.getByTestId("home-inbox-list")).not.toBeVisible();

    // The TL;DR line renders as the summary — not the truncated body, and
    // never raw markdown.
    const tldrLine = page
      .getByTestId("catchup-line")
      .filter({ hasText: "Relay migration is complete and green." });
    await expect(tldrLine).toBeVisible();
    await expect(tldrLine).not.toContainText("TL;DR");
    await expect(tldrLine).not.toContainText("**");
    await expect(tldrLine).not.toContainText("Long-form detail");

    // Attention asks are counted, never listed as lines.
    const needsCount = page.getByTestId("catchup-needs-count");
    await expect(needsCount).toBeVisible();
    await expect(needsCount).toContainText("need you");
    await expect(
      page
        .getByTestId("catchup-line")
        .filter({ hasText: "Please review the release checklist." }),
    ).toHaveCount(0);

    // Opening Catch up moved no read boundary.
    if (badgeBefore !== null) {
      await expect(page.getByTestId("sidebar-home-count")).toHaveText(
        badgeBefore,
      );
    }

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/01-catch-up.png` });

    // Every line deep links to its own message.
    await tldrLine.click();
    await page.waitForURL(/\/channels\//);
    await page.goBack();
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();

    // The needs-you count links to the Attention surface.
    await page.getByTestId("inbox-catchup-toggle").click();
    await expect(page.getByTestId("catchup-view")).toBeVisible();
    await page.getByTestId("catchup-needs-count").click();
    await page.waitForURL(/\/attention/);
    await page.goBack();
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();

    // Mark all as read clears the reading to the empty state.
    await page.getByTestId("inbox-catchup-toggle").click();
    await expect(page.getByTestId("catchup-view")).toBeVisible();
    await page.getByTestId("catchup-mark-all-read").click();
    await expect(page.getByTestId("catchup-empty")).toBeVisible();
    await expect(
      page.getByText("Nothing new since you last looked."),
    ).toBeVisible();

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/02-empty.png` });

    // Toggle back returns the conversation list.
    await page.getByTestId("inbox-catchup-toggle").click();
    await expect(page.getByTestId("catchup-view")).not.toBeVisible();
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  });
});
