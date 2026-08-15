import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

/**
 * Opening a thread lands on the reader's first unread reply.
 *
 * The assertion that carries the behaviour is "the divider is inside the
 * replies viewport with no scrolling of our own". `thread-unread.spec.ts`
 * already proves the divider *exists*; before this change reaching it needed
 * an explicit `scrollIntoViewIfNeeded()`, because the panel bottom-pinned past
 * every unread reply. Asserting visibility alone would still pass in that
 * world, so these tests never scroll before measuring.
 */

const SHOT_DIR = "test-results/thread-resume";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () =>
      page.evaluate(
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
      ),
    )
    .toBe(true);
}

async function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  options: { parentEventId?: string; pubkey?: string; createdAt?: number } = {},
) {
  const event = await page.evaluate(
    ({ ch, msg, parentEventId, pubkey, ts }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string;
            pubkey?: string;
            createdAt?: number;
          }) => { id: string; created_at: number; pubkey: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId,
        pubkey,
        createdAt: ts,
      }),
    {
      ch: channelName,
      msg: content,
      parentEventId: options.parentEventId,
      pubkey: options.pubkey ?? TEST_IDENTITIES.alice.pubkey,
      ts: options.createdAt,
    },
  );
  if (!event) throw new Error("Mock message emitter is not installed");
  return event;
}

// Unread replies must be dated strictly after the frontier captured when the
// thread was last open.
function unreadTimestamp() {
  return Math.floor(Date.now() / 1000) + 60;
}

/** Reads a thread once, so its replies carry a read frontier, then closes it. */
async function establishReadFrontier(page: import("@playwright/test").Page) {
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();
}

test.describe("thread resume at last read", () => {
  test("01-thread-resume-opens-at-first-unread", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await emitMockMessage(page, "general", "Earlier reply, already read", {
      parentEventId: "mock-general-welcome",
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });

    await establishReadFrontier(page);

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // The reply list must overflow the panel by a wide margin. If every reply
    // fits, a bottom-pinned panel shows the divider too and the assertion
    // below passes without the resume — the test would prove nothing.
    const base = unreadTimestamp();
    for (let i = 0; i < 40; i++) {
      await emitMockMessage(page, "general", `Unread reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        createdAt: base + i,
      });
    }

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();

    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toBeVisible();

    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();

    // Captured before asserting, so the same spec run against a build without
    // the resume still yields a comparable "where did it land" frame.
    await waitForAnimations(page);
    await panel.screenshot({
      path: `${SHOT_DIR}/01-thread-resume-opens-at-first-unread.png`,
    });

    // The behaviour under test: no scrollIntoViewIfNeeded before measuring.
    // `toBeInViewport` honours clipping by scrolling ancestors, so a divider
    // scrolled out of the replies list correctly reads as not in viewport.
    await expect(divider).toBeInViewport();
  });

  test("02-fully-read-thread-still-opens-at-newest", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    const base = Math.floor(Date.now() / 1000) - 60;
    for (let i = 0; i < 14; i++) {
      await emitMockMessage(page, "general", `Read reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        createdAt: base + i,
      });
    }

    // Read the whole thread, then come back to it with nothing new.
    await establishReadFrontier(page);
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();

    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toBeVisible();

    // Nothing unread means no divider and no resume: today's bottom pin stands,
    // so the newest reply is the one on screen.
    await expect(page.getByTestId("message-unread-divider")).toHaveCount(0);

    const newest = page
      .getByTestId("message-thread-replies")
      .getByTestId("message-row")
      .last();
    await expect(newest).toBeInViewport();

    await waitForAnimations(page);
    await panel.screenshot({
      path: `${SHOT_DIR}/02-fully-read-thread-still-opens-at-newest.png`,
    });
  });
});
