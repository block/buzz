/**
 * Screenshots documenting the thread-notification traceability design for
 * PR #3754. Keeping them in a spec means the next design round regenerates
 * the images from the shipped code instead of letting mocks drift.
 *
 * Run: pnpm build:e2e && pnpm exec playwright test --project=smoke \
 *        tests/e2e/thread-unread-screenshots.spec.ts
 * Output: test-results/thread-unread-screenshots/
 */
import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/thread-unread-screenshots";

// The pubkey the mock bridge logs in as (mirrors `e2eBridge`'s self identity).
const SELF_PUBKEY = "deadbeef".repeat(8);

// Unread thread replies must be dated strictly after the read frontier
// captured when the thread was last open (see thread-unread.spec.ts).
const UNREAD_OFFSET_SECONDS = 60;

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      );
    })
    .toBe(true);
}

async function emitMockMessage(
  page: import("@playwright/test").Page,
  input: {
    channelName: string;
    content: string;
    kind?: number;
    parentEventId?: string;
    pubkey?: string;
    createdAt?: number;
    mentionPubkeys?: string[];
  },
): Promise<{ id: string }> {
  const event = await page.evaluate(
    (message) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (payload: typeof message) => {
            id: string;
          };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.(message),
    { pubkey: TEST_IDENTITIES.alice.pubkey, ...input },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
  return event;
}

test.describe("thread unread design screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("01 — sidebar signal grammar", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    for (const ch of ["random", "agents", "engineering", "alice-tyler"]) {
      await waitForMockLiveSubscription(page, ch);
    }

    // Ambient unread → dot.
    await emitMockMessage(page, {
      channelName: "agents",
      content: "Nightly harness run finished clean.",
      kind: 40002,
    });
    // Mention → solid count.
    await emitMockMessage(page, {
      channelName: "random",
      content: "@tyler can you review the navigation states?",
      kind: 40002,
      mentionPubkeys: [SELF_PUBKEY],
    });
    // DM → solid count.
    await emitMockMessage(page, {
      channelName: "alice-tyler",
      content: "Quick look before the review?",
    });
    // Reply in a self-authored thread → thread-only badge.
    const root = await emitMockMessage(page, {
      channelName: "engineering",
      content: "Proposal: make notification destinations traceable",
      kind: 40002,
      pubkey: SELF_PUBKEY,
    });
    await emitMockMessage(page, {
      channelName: "engineering",
      content: "Replied in the thread with interaction notes.",
      kind: 40002,
      parentEventId: root.id,
    });

    await expect(
      page.getByTestId("channel-thread-unread-engineering"),
    ).toBeVisible();
    await expect(page.getByTestId("channel-unread-random")).toBeVisible();
    await expect(page.getByTestId("channel-unread-alice-tyler")).toBeVisible();
    await expect(page.getByTestId("channel-unread-dot-agents")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({ path: `${SHOTS}/01-sidebar-signal-grammar.png` });
    await page.screenshot({
      clip: { height: 760, width: 272, x: 0, y: 0 },
      path: `${SHOTS}/02-sidebar-signal-grammar-detail.png`,
    });
  });

  test("02 — channel wayfinding and destination", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Establish a read frontier on the welcome thread, then leave.
    await emitMockMessage(page, {
      channelName: "general",
      content: "Initial thread context",
      createdAt: Math.floor(Date.now() / 1000) - 10,
      parentEventId: "mock-general-welcome",
    });
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("auxiliary-panel-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // Three unread replies land in the thread while the user is away.
    const base = Math.floor(Date.now() / 1000) + UNREAD_OFFSET_SECONDS;
    const replies = [
      "The signal hierarchy is the main problem — channel outranks Inbox.",
      "Added before/after states and a thread-specific destination cue.",
      "Deep-linking already works, so this is presentation, not routing.",
    ];
    for (const [index, content] of replies.entries()) {
      await emitMockMessage(page, {
        channelName: "general",
        content,
        createdAt: base + index,
        parentEventId: "mock-general-welcome",
      });
    }

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("thread-unread-badge")).toContainText(
      "3 new",
    );
    await expect(page.getByTestId("thread-unread-accent")).toBeVisible();
    const threadPill = page.getByTestId("thread-unread-pill");
    await expect(threadPill).toContainText("3 new replies in threads");
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/03-channel-thread-cues.png` });

    // The pill jumps to the parent and opens the thread on the New divider.
    await threadPill.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await expect(page.getByTestId("message-unread-divider")).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/04-thread-destination.png` });
  });
});
