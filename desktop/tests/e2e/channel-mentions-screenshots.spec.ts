import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/pr-screenshots";
const VIEWPORT = { width: 1280, height: 720 };

test.use({ viewport: VIEWPORT });

type Padding = { x: number; y: number };

/** Union of several element boxes, padded and clamped to the viewport. */
async function paddedClip(locators: Locator[], padding: Padding) {
  const boxes = await Promise.all(
    locators.map((locator) => locator.boundingBox()),
  );
  const present = boxes.filter(
    (box): box is NonNullable<typeof box> => box !== null,
  );
  expect(present.length).toBe(locators.length);

  const left = Math.min(...present.map((box) => box.x));
  const top = Math.min(...present.map((box) => box.y));
  const right = Math.max(...present.map((box) => box.x + box.width));
  const bottom = Math.max(...present.map((box) => box.y + box.height));

  const x = Math.max(0, left - padding.x);
  const y = Math.max(0, top - padding.y);
  return {
    x,
    y,
    width: Math.min(VIEWPORT.width, right + padding.x) - x,
    height: Math.min(VIEWPORT.height, bottom + padding.y) - y,
  };
}

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
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

function emitNotifyMessage(
  page: Page,
  channelName: string,
  content: string,
  notifyMode: "channel" | "here",
  author: keyof typeof TEST_IDENTITIES = "alice",
) {
  return page.evaluate(
    ({ ch, msg, pubkey, mode }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
            extraTags?: string[][];
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        pubkey,
        extraTags: [["notify", mode]],
      });
    },
    {
      ch: channelName,
      msg: content,
      pubkey: TEST_IDENTITIES[author].pubkey,
      mode: notifyMode,
    },
  );
}

async function openChannel(page: Page, channelName: string) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${channelName}`).click();
}

test("pr: @channel autocomplete row in the composer", async ({ page }) => {
  await openChannel(page, "general");

  const composer = page.getByTestId("message-composer");
  await composer.getByTestId("message-input").fill("Heads up @ch");

  const autocomplete = composer.getByTestId("mention-autocomplete");
  const channelRow = autocomplete.getByTestId("mention-suggestion-channel");
  await expect(channelRow).toContainText("@channel");
  await expect(channelRow).toContainText("Notify everyone in this channel");

  await waitForAnimations(page);
  // Horizontal bounds come from the composer alone — the popup is inset inside
  // it, so padding sideways would drag in the timeline behind the overlay.
  await page.screenshot({
    clip: await paddedClip([autocomplete, composer], { x: 0, y: 10 }),
    path: `${SHOTS}/01-autocomplete.png`,
  });
});

test("pr: @channel send confirmation dialog", async ({ page }) => {
  await openChannel(page, "general");

  const input = page
    .getByTestId("message-composer")
    .getByTestId("message-input");
  await input.fill("@channel standup moved to 10am");
  // Dismiss the autocomplete so Enter sends instead of accepting a suggestion.
  await input.press("Escape");
  await input.press("Enter");

  const dialog = page.getByRole("alertdialog");
  await expect(page.getByTestId("channel-notify-confirm")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    clip: await paddedClip([dialog], { x: 24, y: 24 }),
    path: `${SHOTS}/02-confirm-dialog.png`,
  });
});

test("pr: @channel and @here chips in the timeline", async ({ page }) => {
  await openChannel(page, "general");
  await waitForMockLiveSubscription(page, "general");

  await emitNotifyMessage(
    page,
    "general",
    "@channel deploy freeze starts at noon",
    "channel",
  );
  const channelRow = page
    .getByTestId("message-row")
    .filter({ hasText: "deploy freeze starts at noon" })
    .first();
  await expect(channelRow.locator("[data-mention]")).toHaveText("@channel");

  await waitForAnimations(page);
  await page.screenshot({
    clip: await paddedClip([channelRow], { x: 24, y: 10 }),
    path: `${SHOTS}/03-pill-render.png`,
  });

  // A different author breaks message grouping, so the @here row renders its
  // own avatar + header instead of a bare continuation line.
  await emitNotifyMessage(
    page,
    "general",
    "@here anyone online to review the release notes?",
    "here",
    "bob",
  );
  const hereRow = page
    .getByTestId("message-row")
    .filter({ hasText: "anyone online to review the release notes?" })
    .first();
  await expect(hereRow.locator("[data-mention]")).toHaveText("@here");

  await waitForAnimations(page);
  await page.screenshot({
    clip: await paddedClip([hereRow], { x: 24, y: 10 }),
    path: `${SHOTS}/04-here-pill.png`,
  });
});
