import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const SHOTS = "test-results/channel-row-decoration-pr";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, kind: k }) => {
          return (
            (
              window as Window & {
                __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                  kind?: number;
                }) => boolean;
              }
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: currentChannelName,
              kind: k,
            }) ?? false
          );
        },
        { currentChannelName: channelName, kind },
      );
    })
    .toBe(true);
}

async function getBadgeState(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const w = window as Window & {
      __BUZZ_E2E_APP_BADGE_STATE__?: string;
      __BUZZ_E2E_APP_BADGE_COUNT__?: number;
    };
    return {
      state: w.__BUZZ_E2E_APP_BADGE_STATE__ ?? "none",
      count: w.__BUZZ_E2E_APP_BADGE_COUNT__ ?? 0,
    };
  });
}

async function waitForBadgeState(
  page: import("@playwright/test").Page,
  expected: { state: string; count?: number },
) {
  await expect
    .poll(async () => getBadgeState(page), { timeout: 5_000 })
    .toEqual(
      expect.objectContaining({
        state: expected.state,
        ...(expected.count !== undefined ? { count: expected.count } : {}),
      }),
    );
}

async function getSettledBadgeState(page: import("@playwright/test").Page) {
  // The mock bridge seeds a couple of unread items during app startup. Let
  // those settle before asserting deltas from newly emitted messages.
  await page.waitForTimeout(2000);
  return getBadgeState(page);
}

async function getSidebarHomeBadgeText(page: import("@playwright/test").Page) {
  return page
    .getByTestId("sidebar-home-count")
    .allTextContents()
    .then((texts) => texts[0] ?? null);
}

function withAdditionalBadgeCount(baseline: { count: number }, count: number) {
  return { state: "count", count: baseline.count + count };
}

function withDotOnlyBadge(baseline: { state: string; count: number }) {
  return baseline.count > 0 ? baseline : { state: "dot", count: 0 };
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("selected Inbox and Agents rows keep their highlight without bold text", async ({
  page,
}) => {
  await page.goto("/");

  const inbox = page
    .getByTestId("sidebar-primary-menu")
    .getByRole("button", { name: "Inbox", exact: true });
  await expect(inbox).toHaveAttribute("data-active", "true");
  await expect(inbox).toHaveCSS("font-weight", "400");

  const agents = page.getByTestId("open-agents-view");
  await agents.click();
  await expect(agents).toHaveAttribute("data-active", "true");
  await expect(agents).toHaveCSS("font-weight", "400");
});

test("hovering a channel keeps its text color", async ({ page }) => {
  await page.goto("/");
  const channel = page.getByTestId("channel-engineering");
  const initialColor = await channel.evaluate(
    (element) => getComputedStyle(element).color,
  );

  await channel.hover();
  await expect(channel).toHaveCSS("color", initialColor);
});

test("direct-message rows become prominent only when unread", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
  });
  await page.goto("/");
  const directMessage = page.getByTestId("channel-alice-tyler");

  await directMessage.click();
  await waitForMockLiveSubscription(page, "alice-tyler");
  await page.getByTestId("channel-general").click();

  const label = directMessage.locator("[data-sidebar-row-label]");
  await expect(directMessage).toHaveCSS("opacity", "1");
  await expect(label).toHaveCSS("opacity", "0.8");
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "An unread direct message",
      kind: 40002,
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(label).toHaveCSS("opacity", "1");
  await expect(directMessage).toHaveCSS("font-weight", "700");
});

test("light mode reserves full opacity for unread text and avatars", async ({
  page,
}) => {
  await page.goto("/");

  const directMessage = page.getByTestId("channel-alice-tyler");
  await directMessage.click();
  await waitForMockLiveSubscription(page, "alice-tyler");
  await page.getByTestId("channel-general").click();

  const inbox = page
    .getByTestId("sidebar-primary-menu")
    .getByRole("button", { name: "Inbox", exact: true });
  await expect(inbox).toHaveCSS("opacity", "1");
  await expect(inbox.locator("[data-sidebar=menu-label]")).toHaveCSS(
    "opacity",
    "0.8",
  );
  await expect(inbox.locator("svg")).toHaveCSS("opacity", "0.8");
  await expect(directMessage).toHaveCSS("opacity", "1");
  await expect(directMessage.locator("[data-sidebar-row-label]")).toHaveCSS(
    "opacity",
    "0.8",
  );

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "An unread direct message in light mode",
      kind: 40002,
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(directMessage.locator("[data-sidebar-row-label]")).toHaveCSS(
    "opacity",
    "1",
  );
  await expect(directMessage).toHaveCSS("font-weight", "700");
});

test("dark mode keeps selected labels regular and channel-level unread labels bold", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
  });
  await page.goto("/");

  await expect(page.locator("html")).toHaveClass(/dark/);
  const inbox = page
    .getByTestId("sidebar-primary-menu")
    .getByRole("button", { name: "Inbox", exact: true });
  await expect(inbox).toHaveAttribute("data-active", "true");
  await expect(inbox).toHaveCSS("font-weight", "400");
  await expect(page.getByTestId("open-agents-view")).toHaveCSS("opacity", "1");
  await expect(
    page.getByTestId("open-agents-view").locator("[data-sidebar=menu-label]"),
  ).toHaveCSS("opacity", "0.8");
  await expect(page.getByTestId("open-agents-view").locator("svg")).toHaveCSS(
    "opacity",
    "0.8",
  );

  await page.getByTestId("channel-general").click();
  await expect(inbox).toHaveCSS("opacity", "1");
  await expect(inbox.locator("[data-sidebar=menu-label]")).toHaveCSS(
    "opacity",
    "0.8",
  );
  await waitForMockLiveSubscription(page, "random");
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "random",
      content: "A dark-mode channel-level unread message",
      kind: 40002,
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  const unreadChannel = page.getByTestId("channel-random");
  const engineeringLabel = page
    .getByTestId("channel-engineering")
    .locator("[data-sidebar-row-label]");
  await expect(engineeringLabel).toHaveCSS("opacity", "0.8");
  await expect(
    page.getByTestId("channel-engineering").locator("svg"),
  ).toHaveCSS("opacity", "0.8");
  await expect(unreadChannel.locator("[data-sidebar-row-label]")).toHaveCSS(
    "opacity",
    "1",
  );
  await expect(unreadChannel).toHaveCSS("font-weight", "700");
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/sidebar-dark-unread.png`,
    clip: { x: 0, y: 0, width: 320, height: 720 },
  });
});

test("offscreen top-level unread shows the primary sidebar arrow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 360 });
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await waitForMockLiveSubscription(page, "random");
  await page.getByTestId("channel-general").click();

  const sidebarScroller = page
    .getByTestId("app-sidebar")
    .locator('[data-sidebar="content"]');
  await sidebarScroller.evaluate((element) => {
    const random = element.querySelector<HTMLElement>(
      '[data-testid="channel-random"]',
    );
    if (!random) throw new Error("Could not find #random in the sidebar");

    // Keep #random just above the viewport so it is the next unread row.
    element.scrollTop +=
      random.getBoundingClientRect().bottom -
      element.getBoundingClientRect().top +
      1;
  });
  await expect(page.getByTestId("channel-random")).not.toBeInViewport();

  await page.evaluate(
    ({ pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "A regular channel message",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  const activityArrow = page.getByTestId("sidebar-more-unread-above");
  await expect(activityArrow).toBeVisible();
  await expect(activityArrow).toHaveClass(/bg-primary/);
  await activityArrow.click();
  await expect(page.getByTestId("channel-random")).toBeInViewport();
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/sidebar-top-level-unread-arrow.png`,
    clip: { x: 0, y: 0, width: 320, height: 360 },
  });
});

test("offscreen unread DM shows the primary sidebar arrow", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await waitForMockLiveSubscription(page, "alice-tyler");
  await page.getByTestId("channel-general").click();
  await page.setViewportSize({ width: 1280, height: 360 });

  const sidebarScroller = page
    .getByTestId("app-sidebar")
    .locator('[data-sidebar="content"]');
  await sidebarScroller.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect(page.getByTestId("channel-alice-tyler")).not.toBeInViewport();

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "An unread direct message",
      kind: 40002,
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  const activityArrow = page.getByTestId("sidebar-more-unread-below");
  await expect(activityArrow).toBeVisible();
  await expect(activityArrow).toHaveClass(/bg-primary/);
});

test("regular message bolds inactive channel without numeric badge", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");
  const baselineBadge = await getSettledBadgeState(page);

  await page.evaluate(
    ({ pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Regular message, no mention",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await expect(page.getByTestId("channel-unread-dot-random")).toHaveCount(0);
  await waitForBadgeState(page, withDotOnlyBadge(baselineBadge));

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("channel-random")).toHaveAttribute(
    "data-active",
    "true",
  );
  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "400",
  );
});

test("top-level @mention bolds the channel without a row badge", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");
  const baselineBadge = await getSettledBadgeState(page);

  await page.evaluate(
    ({ pubkey, mentionPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Hey @tyler check this out",
        kind: 40002,
        pubkey,
        mentionPubkeys: [mentionPubkey],
      });
    },
    {
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkey: DEFAULT_MOCK_PUBKEY,
    },
  );

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await expect(page.getByTestId("channel-unread-dot-random")).toHaveCount(0);
  await waitForBadgeState(page, withAdditionalBadgeCount(baselineBadge, 1));
});

test("numeric badge increments for DM message", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "alice-tyler");
  const baselineBadge = await getSettledBadgeState(page);

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "Hey, got a minute?",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("channel-unread-alice-tyler")).toBeVisible();
  await waitForBadgeState(page, withAdditionalBadgeCount(baselineBadge, 1));
});

test("interested thread reply shows the channel preview dot without incrementing Inbox", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");
  const baselineBadge = await getSettledBadgeState(page);
  const baselineHomeBadge = await getSidebarHomeBadgeText(page);

  const rootEventId = await page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "random",
      content: "Conversation I started",
      kind: 40002,
      pubkey: "deadbeef".repeat(8),
    });
    return root?.id;
  });

  await page.evaluate(
    ({ parentEventId, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Thread reply to a followed conversation",
        kind: 40002,
        parentEventId,
        pubkey,
      });
    },
    { parentEventId: rootEventId, pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await expect(page.getByTestId("channel-unread-dot-random")).toBeVisible();
  await expect
    .poll(() => getSidebarHomeBadgeText(page))
    .toBe(baselineHomeBadge);
  await waitForBadgeState(page, baselineBadge);
});

test("broadcast reply bolds the channel without a thread dot", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");
  const baselineBadge = await getSettledBadgeState(page);

  await page.evaluate(
    ({ pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Broadcast reply to the channel",
        kind: 40002,
        pubkey,
        extraTags: [
          ["broadcast", "1"],
          ["e", "some-root-event-id"],
        ],
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await expect(page.getByTestId("channel-unread-dot-random")).toHaveCount(0);
  await waitForBadgeState(page, withAdditionalBadgeCount(baselineBadge, 1));
});

test("mark-as-read via context menu clears channel unread indicator", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");

  // Wait for catch-up to settle, then record baseline badge state
  // (other mock channels may have pre-existing unreads from seeded history)
  await page.waitForTimeout(2000);
  const baselineBadge = await getBadgeState(page);

  await page.evaluate(
    ({ pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Message to be marked read",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);

  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByText("Mark as read").click();

  await expect(page.getByTestId("channel-random")).not.toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await waitForBadgeState(page, baselineBadge);
});

test("mark-as-unread via context menu bolds the channel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  const baselineBadge = await getSettledBadgeState(page);

  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByText("Mark unread").click();

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await expect(page.getByTestId("channel-unread-dot-random")).toHaveCount(0);
  await waitForBadgeState(page, withAdditionalBadgeCount(baselineBadge, 1));
});

test("marking a message unread bolds its channel after leaving", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await waitForMockLiveSubscription(page, "random");

  const message = await page.evaluate(
    ({ pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Keep this channel message unread",
        kind: 40002,
        pubkey,
      }),
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );
  if (!message) {
    throw new Error("Mock message emitter is unavailable");
  }

  const messageRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Keep this channel message unread" });
  await expect(messageRow).toBeVisible();
  await messageRow.hover();
  await page.getByTestId(`more-actions-${message.id}`).click();
  const toggle = page.getByTestId(`mark-read-toggle-${message.id}`);
  await expect(toggle).toHaveText("Mark unread");
  await toggle.click();

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/sidebar-active-manual-unread.png`,
    clip: { x: 0, y: 0, width: 320, height: 720 },
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-dot-random")).toHaveCount(0);
});

test("remote read-state rollback is ignored while local mark-unread still increments badge", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Baseline: random has no unread dot
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  const baselineBadge = await getSettledBadgeState(page);

  // Wait for ReadStateManager's live subscription (kind:30078) to be
  // established before injecting events.
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
                kind?: number;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
            kind: 30078,
          }) ?? false
        );
      });
    })
    .toBe(true);

  const REMOTE_CLIENT_ID = "other-device-client-id";
  const REMOTE_SLOT_ID = "e2e00000000000000000000000000000";
  const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
  const now = Math.floor(Date.now() / 1000);

  // Step 1: seed a "read at now" state from the remote device so the
  // local manager has a baseline value for this channel context.
  await page.evaluate(
    ({ clientId, slotId, channelId, ts }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_READ_STATE__?: (input: {
            clientId: string;
            contexts: Record<string, number>;
            createdAt: number;
            slotId: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_READ_STATE__?.({
        clientId,
        slotId,
        contexts: { [channelId]: ts },
        createdAt: ts,
      });
    },
    {
      clientId: REMOTE_CLIENT_ID,
      slotId: REMOTE_SLOT_ID,
      channelId: RANDOM_CHANNEL_ID,
      ts: now,
    },
  );

  // Step 2: a remote rollback carries an older read timestamp in a newer
  // event. NIP-RS read markers are monotonic, so this must be ignored.
  await page.evaluate(
    ({ clientId, slotId, channelId, ts, createdAt }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_READ_STATE__?: (input: {
            clientId: string;
            contexts: Record<string, number>;
            createdAt: number;
            slotId: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_READ_STATE__?.({
        clientId,
        slotId,
        contexts: { [channelId]: ts },
        createdAt,
      });
    },
    {
      clientId: REMOTE_CLIENT_ID,
      slotId: REMOTE_SLOT_ID,
      channelId: RANDOM_CHANNEL_ID,
      ts: now - 100,
      createdAt: now + 5,
    },
  );

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);

  // Local mark-unread remains an in-session affordance and should still bold
  // the channel immediately without publishing a lower read timestamp.
  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByText("Mark unread").click();
  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "700",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await waitForBadgeState(page, withAdditionalBadgeCount(baselineBadge, 1));

  // Step 3: remote advance clears the local forced-unread dot.
  await page.evaluate(
    ({ clientId, slotId, channelId, ts, createdAt }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_READ_STATE__?: (input: {
            clientId: string;
            contexts: Record<string, number>;
            createdAt: number;
            slotId: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_READ_STATE__?.({
        clientId,
        slotId,
        contexts: { [channelId]: ts },
        createdAt,
      });
    },
    {
      clientId: REMOTE_CLIENT_ID,
      slotId: REMOTE_SLOT_ID,
      channelId: RANDOM_CHANNEL_ID,
      ts: now + 10,
      createdAt: now + 10,
    },
  );

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
});
