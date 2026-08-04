import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

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

const SHOTS = "test-results/attention";

const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents
const AGENT_PUBKEY =
  "db0b028cd36f4d3e36c8300cce87252c1f7fc9495ffecc53f393fcac341ffd36";

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

/**
 * Seed the mock feed with the injected items the redesigned view needs on
 * top of the built-in feed (a mention with no reader-addressed ask + a
 * 40007 reminder): a 46010 approval that has been waiting three days
 * (overdue section) and an ask-less FYI mention (To note section).
 */
async function seedAttentionFeed(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () =>
      typeof (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );
  await page.evaluate(
    ({ agentPubkey, channelId, viewerPubkey }) => {
      const push = (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      push?.({
        category: "needs_action",
        channel_id: channelId,
        channel_name: "agents",
        content:
          "Release workflow paused: approve the desktop deploy to staging?",
        created_at: Math.floor(Date.now() / 1_000) - 3 * 86_400,
        id: "mock-feed-attention-approval",
        kind: 46010,
        pubkey: agentPubkey,
        tags: [["p", viewerPubkey]],
      });
      push?.({
        category: "mention",
        channel_id: channelId,
        channel_name: "agents",
        content: "FYI the launch deck shipped last night.",
        created_at: Math.floor(Date.now() / 1_000) - 300,
        id: "mock-feed-attention-fyi",
        kind: 9,
        pubkey: agentPubkey,
        tags: [["p", viewerPubkey]],
      });
      // Tier-1 declared ask with a closed option set (base_prompt.md
      // convention). The viewer's mock display name is "tyler".
      push?.({
        category: "mention",
        channel_id: channelId,
        channel_name: "agents",
        content: [
          "**Needs Tyler, decision:** Ship the attention release now or wait for the relay fix?",
          "- Ship it now.",
          "- Wait for the relay fix.",
          "- Need more detail before deciding.",
        ].join("\n"),
        created_at: Math.floor(Date.now() / 1_000) - 120,
        id: "mock-feed-attention-declared-decision",
        kind: 9,
        pubkey: agentPubkey,
        tags: [["p", viewerPubkey]],
      });
      // Two declared asks for the same viewer -> multi-ask card.
      push?.({
        category: "mention",
        channel_id: channelId,
        channel_name: "agents",
        content: [
          "**Needs Tyler, question:** Did the overnight backup complete?",
          "**Needs Tyler, review:** Review the retention change when you can.",
        ].join("\n"),
        created_at: Math.floor(Date.now() / 1_000) - 60,
        id: "mock-feed-attention-declared-multi",
        kind: 9,
        pubkey: agentPubkey,
        tags: [["p", viewerPubkey]],
      });
      // Declared blocked ask: the one type whose primary action is Done.
      push?.({
        category: "mention",
        channel_id: channelId,
        channel_name: "agents",
        content:
          "**Needs Tyler, blocked:** I need the staging credentials before I can deploy.",
        created_at: Math.floor(Date.now() / 1_000) - 30,
        id: "mock-feed-attention-declared-blocked",
        kind: 9,
        pubkey: agentPubkey,
        tags: [["p", viewerPubkey]],
      });
    },
    {
      agentPubkey: AGENT_PUBKEY,
      channelId: AGENTS_CHANNEL_ID,
      viewerPubkey: TEST_IDENTITIES.tyler.pubkey,
    },
  );
}

test.describe("attention views", () => {
  test("action-driven cards: sections, expand, quick options, undo, posting", async ({
    page,
  }) => {
    // Run as tyler so the declared-ask tier can match "Needs Tyler, …"
    // against the viewer's mock display name. Must precede the bridge.
    await seedActiveIdentity(page, TEST_IDENTITIES.tyler);
    await enableAttentionFeature(page);
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });

    const navItem = page.getByTestId("open-attention-view");
    await expect(navItem).toBeVisible();
    await seedAttentionFeed(page);
    await navItem.click();

    const view = page.getByTestId("attention-view");
    await expect(view).toBeVisible();

    // 6 real asks (approval, tyler mention, tyler reminder, declared
    // decision, declared multi, declared blocked) + 1 to note (the seeded
    // FYI mention).
    const cards = page.locator('article[data-testid^="attention-card-"]');
    await expect(cards).toHaveCount(7);

    // Split Needs Me count: real asks and to-note are counted separately.
    const needsTab = page.getByTestId("attention-tab-needsMe");
    await expect(needsTab).toContainText("6 need you");
    await expect(needsTab).toContainText("1 to note");

    // Sections: the 3-day-old approval is overdue, the reminder is today,
    // the ask-less mentions sit under To note.
    await expect(page.getByTestId("attention-section-overdue")).toBeVisible();
    await expect(page.getByTestId("attention-section-today")).toBeVisible();
    await expect(page.getByTestId("attention-section-note")).toBeVisible();
    await expect(
      page.getByTestId("attention-section-overdue").locator("article"),
    ).toHaveCount(1);

    // Badges vary by ask type: Approval plus at least one other type, and
    // the declared decision message gets the Decision badge.
    const badges = page.getByTestId("attention-badge");
    await expect(badges.filter({ hasText: "Approval" })).toHaveCount(1);
    await expect(badges.filter({ hasText: "Decision" })).toHaveCount(1);
    await expect(badges.filter({ hasText: "Review" }).first()).toBeVisible();
    await expect(badges.filter({ hasText: "Heads up" })).toHaveCount(1);
    await expect(badges.filter({ hasText: "Blocked" })).toHaveCount(1);

    // Multi-declaration message headlines the ask count, never one ask.
    await expect(
      page.getByTestId("attention-card-ask").filter({ hasText: "2 asks" }),
    ).toBeVisible();

    // The headline is the ask, not the sender.
    const firstAsk = cards.first().getByTestId("attention-card-ask");
    await expect(firstAsk).toContainText("approve the desktop deploy");
    const askText = (await firstAsk.textContent())?.trim() ?? "";
    expect(askText.length).toBeGreaterThan(0);
    expect(askText).not.toMatch(/^(Tyler|Alice|Bob)$/);
    // Staleness is surfaced on the meta line.
    await expect(cards.first()).toContainText("waiting 3 days");

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/01-needs-me-redesign.png` });

    // Keyboard: j selects the first card, e expands it.
    await view.focus();
    await page.keyboard.press("j");
    await expect(cards.first()).toHaveAttribute("data-selected", "true");
    await page.keyboard.press("e");
    const expanded = page.getByTestId("attention-card-expanded");
    await expect(expanded).toBeVisible();

    // Full message, quick-select options (approval pair + the way out),
    // and a reply box. Options are scoped to the card: collapsed cards
    // now show their own option rows too.
    await expect(expanded).toContainText(
      "Release workflow paused: approve the desktop deploy to staging?",
    );
    const quickOptions = cards.first().getByTestId("attention-quick-option");
    await expect(quickOptions).toHaveCount(3);
    await expect(quickOptions.first()).toHaveText("Approve");
    await expect(quickOptions.last()).toHaveText(
      "Not enough detail yet, tell me more",
    );
    await expect(page.getByTestId("attention-reply-input")).toBeFocused();
    await expect(page.getByTestId("attention-action-reply")).toBeDisabled();

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/02-expanded-card.png` });

    // To note section close-up (scoped shot keeps the three PNGs distinct).
    await waitForAnimations(page);
    await page
      .getByTestId("attention-section-note")
      .screenshot({ path: `${SHOTS}/03-to-note.png` });

    // Collapse again to keep the list stable for the action flows.
    await view.focus();
    await page.keyboard.press("e");
    await expect(expanded).not.toBeVisible();

    // Declared options are visible on the COLLAPSED card (one click clears
    // the row without an expand), and clicking one flows through the reply
    // queue (posts the option text verbatim after the undo window).
    const decisionCard = page.locator(
      'article[data-testid^="attention-card-"]',
      { hasText: "Ship the attention release now" },
    );
    const declaredOptions = decisionCard.getByTestId("attention-quick-option");
    await expect(declaredOptions).toHaveCount(3);
    await expect(declaredOptions.first()).toHaveText("Ship it now.");
    await declaredOptions.first().click();
    await expect(cards).toHaveCount(6);
    await expect(page.getByText("Reply queued — posts in 5s")).toBeVisible();
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(cards).toHaveCount(7);

    // Multi-ask: the collapsed card lists its sub-asks (never just a
    // count), and the expanded view answers through per-ask sub-items.
    // The optionless Review sub-ask gets its default pair (fixture 4).
    const multiCard = page.locator('article[data-testid^="attention-card-"]', {
      hasText: "2 asks",
    });
    await expect(
      multiCard.getByTestId("attention-collapsed-subasks"),
    ).toContainText("Did the overnight backup complete?");
    await multiCard.getByTestId("attention-card-ask").click();
    await expect(multiCard.getByTestId("attention-subitem")).toHaveCount(2);
    await expect(
      multiCard.getByTestId("attention-subitems-progress"),
    ).toHaveText("0 of 2 answered");
    await expect(
      multiCard.getByTestId("attention-subitem-option").filter({
        hasText: "Looks good",
      }),
    ).toBeVisible();
    await multiCard.getByTestId("attention-card-ask").click();

    // Noted on a To note item is LOCAL-ONLY: the card leaves immediately,
    // nothing publishes — no reply-queue "posts in 5s" toast appears.
    const notedCard = page.locator('article[data-testid^="attention-card-"]', {
      hasText: "FYI the launch deck shipped",
    });
    await notedCard.hover();
    await notedCard.getByTestId("attention-action-noted").click();
    await expect(cards).toHaveCount(6);
    await expect(page.getByText("Noted locally")).toBeVisible();
    await expect(page.getByText("Noted — reply posts in 5s")).toHaveCount(0);
    const undoButton = page.getByRole("button", { name: "Undo" });
    await expect(undoButton).toBeVisible();
    await undoButton.click();
    await expect(cards).toHaveCount(7);

    // Note all clears the whole To note strip locally; Undo restores it.
    await expect(page.getByTestId("attention-section-note")).toBeVisible();
    await page.getByTestId("attention-note-all").click();
    await expect(page.getByTestId("attention-section-note")).not.toBeVisible();
    await expect(cards).toHaveCount(6);
    await expect(page.getByText("Noted 1 item locally")).toBeVisible();
    await page
      .locator("li", { hasText: "Noted 1 item locally" })
      .getByRole("button", { name: "Undo" })
      .click();
    await expect(page.getByTestId("attention-section-note")).toBeVisible();
    await expect(cards).toHaveCount(7);

    // Locked action matrix: Done only where it means something. The
    // Blocked card has Done as primary and no overflow; the reminder
    // (Review) card has no Done and offers Handled elsewhere instead.
    const blockedCard = page.locator(
      'article[data-testid^="attention-card-"]',
      { hasText: "staging credentials" },
    );
    await blockedCard.hover();
    await expect(
      blockedCard.getByTestId("attention-action-done"),
    ).toBeVisible();
    await expect(
      blockedCard.getByTestId("attention-action-overflow"),
    ).toHaveCount(0);

    // Handled elsewhere: held like every action, publishes the shared
    // honest line exactly once after the 5s undo window.
    const reminderCard = page.locator(
      'article[data-testid^="attention-card-"]',
      {
        hasText: "answer Bob in the launch DM thread",
      },
    );
    await reminderCard.hover();
    await expect(reminderCard.getByTestId("attention-action-done")).toHaveCount(
      0,
    );
    await reminderCard.getByTestId("attention-action-overflow").click();
    await reminderCard
      .getByTestId("attention-action-handled-elsewhere")
      .click();
    await expect(cards).toHaveCount(6);
    await expect(
      page.getByText("Handled elsewhere — reply posts in 5s"),
    ).toBeVisible();
    await page.waitForTimeout(5_500);
    await expect(cards).toHaveCount(6);
    await expect(
      page.getByText("Could not post your reply", { exact: false }),
    ).not.toBeVisible();
    await page.getByTestId("attention-tab-done").click();
    await expect(
      page.locator('article[data-testid^="attention-card-"]', {
        hasText: "answer Bob in the launch DM thread",
      }),
    ).toBeVisible();

    // Deep link: opening a card lands in the original channel conversation.
    await page.getByTestId("attention-tab-needsMe").click();
    const openTarget = cards.first();
    await openTarget.hover();
    await openTarget.getByTestId("attention-action-open").click();
    await page.waitForURL(/\/channels\//);
  });

  test("empty waiting view explains itself", async ({ page }) => {
    await enableAttentionFeature(page);
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.getByTestId("open-attention-view").click();
    await expect(page.getByTestId("attention-view")).toBeVisible();
    await page.getByTestId("attention-tab-waiting").click();
    await expect(page.getByTestId("attention-empty-state")).toBeVisible();
    await expect(
      page.getByText(
        "Nothing parked. Items you are waiting on others for land here.",
      ),
    ).toBeVisible();
  });
});
