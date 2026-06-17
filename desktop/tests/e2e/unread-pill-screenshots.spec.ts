import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/unread-pill";

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

function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  createdAt?: number,
) {
  return page.evaluate(
    ({ ch, msg, pubkey, ts }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
            createdAt?: number;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        pubkey,
        createdAt: ts,
      });
    },
    {
      ch: channelName,
      msg: content,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      ts: createdAt,
    },
  );
}

// Unread messages must be created strictly after the read frontier captured
// when the channel was last open. The frontier is captured at the current
// second on open, and computeChannelUnreadMarker uses a strict
// `createdAt > frontier` predicate — so emitting at the same wall-clock second
// leaves the messages on the read side and the pill/divider never render.
// Dating them a minute ahead puts them deterministically past the frontier.
const UNREAD_OFFSET_SECONDS = 60;

function unreadTimestamp() {
  return Math.floor(Date.now() / 1000) + UNREAD_OFFSET_SECONDS;
}

// Emit `count` unread messages to general, staggered one second apart so they
// sort deterministically and all land strictly past the read frontier.
async function emitUnreadMessages(
  page: import("@playwright/test").Page,
  count: number,
) {
  const base = unreadTimestamp();
  for (let index = 0; index < count; index += 1) {
    await emitMockMessage(
      page,
      "general",
      `Unread message ${index + 1}`,
      base + index,
    );
  }
}

// Scroll the timeline up so the viewport is no longer pinned to the bottom.
// The pill auto-dismisses once the user reaches the bottom of the timeline, so
// it only stays rendered while scrolled up — which is the state these shots
// need to capture. Scrolling part-way (rather than to the very top) keeps real
// message context on screen instead of the channel's empty-state intro.
async function scrollTimelineUp(page: import("@playwright/test").Page) {
  await page.getByTestId("message-timeline").evaluate((el) => {
    el.scrollTop = Math.floor(el.scrollHeight * 0.35);
  });
  await page.waitForTimeout(300);
}

test.describe("unread pill & divider screenshots", () => {
  test("01-unread-pill-visible", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Open general, then switch to random so general becomes inactive
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    await emitUnreadMessages(page, 20);

    // Switch back to general — pill should appear.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Scroll up so the unreads sit below the fold: the pill is the
    // "jump to oldest unread" affordance and only stays on screen while the
    // user is scrolled away from the bottom of the timeline.
    await scrollTimelineUp(page);

    const pill = page.getByTestId("message-unread-pill");
    await expect(pill).toBeVisible();
    await expect(pill).toContainText("20 new messages");

    await page.screenshot({
      path: `${SHOTS}/01-unread-pill-visible.png`,
    });
  });

  test("02-unread-divider-visible", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    await emitUnreadMessages(page, 3);

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();

    // Scroll the divider into view for a clear screenshot
    await divider.scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);

    await page.screenshot({
      path: `${SHOTS}/02-unread-divider-visible.png`,
    });
  });

  test("03-pill-dismissed-after-scroll", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    await emitUnreadMessages(page, 20);

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Scroll up so the pill is showing, matching scenario 01's starting state.
    await scrollTimelineUp(page);

    const pill = page.getByTestId("message-unread-pill");
    await expect(pill).toBeVisible();

    // Click the pill to jump to the oldest unread, which dismisses it.
    await pill.click();

    // Pill should be dismissed
    await expect(pill).toHaveCount(0);

    // Divider should still be visible
    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/03-pill-dismissed-after-scroll.png`,
    });
  });

  test("04-mark-unread-suppresses-pill", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Mark channel unread via context menu on the sidebar item
    await page.getByTestId("channel-general").click({ button: "right" });
    await page.getByText("Mark unread").click();

    // Switch away and back to re-open the channel
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // The unread indicator only renders on inactive channels, so it appears
    // once general is no longer the active channel.
    await expect(page.getByTestId("channel-unread-general")).toBeVisible();

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Pill and divider should NOT appear (suppressed for forced-unread)
    await expect(page.getByTestId("message-unread-pill")).toHaveCount(0);
    await expect(page.getByTestId("message-unread-divider")).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/04-mark-unread-suppresses-pill.png`,
    });
  });
});
