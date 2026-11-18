import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

type MockMessageWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    pubkey?: string;
  }) => { id: string } | undefined;
  __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
    channelName: string;
  }) => boolean;
};

const CHANNEL_NAME = "engineering";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () => {
      return page.evaluate((name) => {
        return (
          (
            window as MockMessageWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }) ??
          false
        );
      }, channelName);
    })
    .toBe(true);
}

test.describe("auxiliary pane close visibility", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  // Regression for #6901: `isolate` on the right auxiliary pane makes it its
  // own stacking context. With no z-index (`auto`, i.e. level 0) the entire
  // pane subtree — including the z-40 header chrome where X/Edit live — paints
  // below the channel's sibling z-30 shared-header backdrop in split layout,
  // washing out the controls (they stay clickable because the backdrop is
  // pointer-events-none, matching the reported videos). The pane's own
  // stacking level must sit above the backdrop for the header to show through.
  test("close button paints above the shared header backdrop in a channel thread", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
    await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL_NAME);
    await waitForMockLiveSubscription(page, CHANNEL_NAME);

    const rootId = await page.evaluate(
      ({ channelName, pubkey }) =>
        (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: "Root message for auxiliary pane close visibility.",
          pubkey,
        })?.id ?? null,
      { channelName: CHANNEL_NAME, pubkey: MOCK_IDENTITY_PUBKEY },
    );
    expect(rootId).not.toBeNull();

    await page.evaluate(
      ({ channelName, parentEventId, pubkey }) => {
        (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: "Reply that opens a split thread panel.",
          parentEventId,
          pubkey,
        });
      },
      {
        channelName: CHANNEL_NAME,
        parentEventId: rootId,
        pubkey: ALICE_PUBKEY,
      },
    );

    const replyButton = page.locator('[data-testid^="reply-message-"]').first();
    await expect(replyButton).toBeVisible();
    await replyButton.click({ force: true });
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await waitForAnimations(page);

    const closeButton = page.getByTestId("auxiliary-panel-close");
    await expect(closeButton).toBeVisible();

    const backdrop = page.getByTestId("channel-shared-header-backdrop");
    await expect(backdrop).toHaveCount(1);

    // The pane is an isolated stacking context (`isolate`) that sits over the
    // channel timeline. Its close button paints correctly only when the pane's
    // own stacking level clears the shared-header backdrop it overlaps. Assert
    // the pane both establishes that context and outranks the backdrop.
    const pane = page.getByTestId("message-thread-panel");
    const [paneIsolation, paneZIndex, backdropZIndex] = await Promise.all([
      pane.evaluate((element) => getComputedStyle(element).isolation),
      pane.evaluate((element) => Number(getComputedStyle(element).zIndex)),
      backdrop.evaluate((element) => Number(getComputedStyle(element).zIndex)),
    ]);

    expect(paneIsolation).toBe("isolate");
    expect(paneZIndex).toBeGreaterThan(backdropZIndex);
  });
});
