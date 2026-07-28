/**
 * Screenshots documenting the device-local "Show join and leave messages"
 * setting: the Settings → Appearance toggle, the default timeline (rows
 * hidden), and the timeline with the setting enabled (rows shown).
 *
 * Run: pnpm build:e2e && pnpm exec playwright test --project=smoke \
 *        tests/e2e/join-leave-visibility-screenshots.spec.ts
 * Output: test-results/join-leave-visibility/
 */
import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { enableJoinLeaveMessages } from "../helpers/joinLeaveMessages";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/join-leave-visibility";

const KIND_SYSTEM_MESSAGE = 40099;

// Skip the 256px sidebar so the timeline fills the shot.
const TIMELINE_CLIP = { x: 256, y: 0, width: 1024, height: 720 };

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

/**
 * Seed a timeline with normal chat messages surrounding membership system
 * rows: alice joins (self-join), bob is added by tyler, then bob leaves.
 */
async function seedTimelineWithMembershipRows(
  page: import("@playwright/test").Page,
) {
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await waitForMockLiveSubscription(page, "random");

  const base = Math.floor(Date.now() / 1000) - 600;
  await page.evaluate(
    ({ alice, bob, tyler, kindSystem, baseTime }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey?: string;
            kind?: number;
            createdAt?: number;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("mock emit unavailable");

      emit({
        channelName: "random",
        content: "Morning! Kicking off the release checklist today.",
        pubkey: tyler,
        createdAt: baseTime,
      });
      emit({
        channelName: "random",
        content: JSON.stringify({
          type: "member_joined",
          actor: alice,
          target: alice,
        }),
        pubkey: alice,
        kind: kindSystem,
        createdAt: baseTime + 60,
      });
      emit({
        channelName: "random",
        content: JSON.stringify({
          type: "member_joined",
          actor: tyler,
          target: bob,
        }),
        pubkey: tyler,
        kind: kindSystem,
        createdAt: baseTime + 120,
      });
      emit({
        channelName: "random",
        content: JSON.stringify({
          type: "member_left",
          actor: bob,
          target: bob,
        }),
        pubkey: bob,
        kind: kindSystem,
        createdAt: baseTime + 180,
      });
      emit({
        channelName: "random",
        content: "Checklist looks good — shipping after lunch.",
        pubkey: alice,
        createdAt: baseTime + 240,
      });
    },
    {
      alice: TEST_IDENTITIES.alice.pubkey,
      bob: TEST_IDENTITIES.bob.pubkey,
      tyler: TEST_IDENTITIES.tyler.pubkey,
      kindSystem: KIND_SYSTEM_MESSAGE,
      baseTime: base,
    },
  );

  await expect(
    page.getByText("Checklist looks good — shipping after lunch."),
  ).toBeVisible();
}

test("capture: settings toggle under Appearance", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "appearance");

  const card = page.getByTestId("settings-message-display");
  await card.scrollIntoViewIfNeeded();
  await expect(
    page.getByTestId("show-join-leave-messages-toggle"),
  ).toBeVisible();
  await expect(
    page.getByTestId("show-join-leave-messages-toggle"),
  ).not.toBeChecked();

  await waitForAnimations(page);
  await card.screenshot({ path: `${SHOTS}/01-settings-toggle.png` });
});

test("capture: timeline hides join/leave rows by default", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await seedTimelineWithMembershipRows(page);

  await expect(page.getByText("joined the channel")).toHaveCount(0);
  await expect(page.getByText("left the channel")).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/02-timeline-default-hidden.png`,
    clip: TIMELINE_CLIP,
  });
});

test("capture: timeline shows join/leave rows when enabled", async ({
  page,
}) => {
  await enableJoinLeaveMessages(page);
  await installMockBridge(page);
  await page.goto("/");
  await seedTimelineWithMembershipRows(page);

  await expect(page.getByText("joined the channel")).toBeVisible();
  await expect(page.getByText("left the channel")).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/03-timeline-enabled-shown.png`,
    clip: TIMELINE_CLIP,
  });
});
