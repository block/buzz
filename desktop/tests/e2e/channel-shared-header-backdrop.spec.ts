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

test.describe("channel shared header backdrop", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("backs a scrolled split auxiliary header above the shared channel backdrop", async ({
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
          content: "Root message for shared header backdrop coverage.",
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

    await page.evaluate(
      ({ channelName, parentEventId, pubkey }) => {
        for (let index = 0; index < 24; index += 1) {
          (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName,
            content: `Scrollable thread reply ${index + 1}. `.repeat(4),
            parentEventId,
            pubkey,
          });
        }
      },
      {
        channelName: CHANNEL_NAME,
        parentEventId: rootId,
        pubkey: ALICE_PUBKEY,
      },
    );

    const threadBody = page.getByTestId("message-thread-body");
    await expect
      .poll(() =>
        threadBody.evaluate(
          (element) => element.scrollHeight > element.clientHeight,
        ),
      )
      .toBe(true);
    await threadBody.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
      element.dispatchEvent(new Event("scroll"));
    });
    await expect
      .poll(() => threadBody.evaluate((element) => element.scrollTop))
      .toBeGreaterThan(0);

    const paneBackdrop = page.getByTestId("auxiliary-panel-header-backdrop");
    await expect(paneBackdrop).toHaveCount(1);

    const sharedBackdrop = page.getByTestId("channel-shared-header-backdrop");
    await expect(sharedBackdrop).toHaveCount(1);

    const chatHeader = page.getByTestId("chat-header");
    const auxiliaryResizeHandle = page.getByTestId(
      "right-auxiliary-pane-resize-handle",
    );

    const [
      hostBox,
      backdropBox,
      paneBackdropBox,
      paneBackdropBackground,
      paneBackdropFilter,
      backdropFilter,
      backdropZIndex,
      headerZIndex,
      resizeHandleZIndex,
      auxiliaryPaneAnimationName,
    ] = await Promise.all([
      page.getByTestId("channel-drop-zone").locator("..").boundingBox(),
      sharedBackdrop.boundingBox(),
      paneBackdrop.boundingBox(),
      paneBackdrop.evaluate(
        (element) => getComputedStyle(element).backgroundColor,
      ),
      paneBackdrop.evaluate(
        (element) => getComputedStyle(element).backdropFilter,
      ),
      sharedBackdrop.evaluate(
        (element) => getComputedStyle(element).backdropFilter,
      ),
      sharedBackdrop.evaluate((element) =>
        Number(getComputedStyle(element).zIndex),
      ),
      chatHeader.evaluate((element) =>
        Number(getComputedStyle(element.parentElement ?? element).zIndex),
      ),
      auxiliaryResizeHandle.evaluate((element) =>
        Number(getComputedStyle(element).zIndex),
      ),
      page
        .getByTestId("message-thread-panel")
        .evaluate((element) => getComputedStyle(element).animationName),
    ]);

    expect(hostBox).not.toBeNull();
    expect(backdropBox).not.toBeNull();
    expect(paneBackdropBox).not.toBeNull();
    expect(Math.round(paneBackdropBox?.y ?? 0)).toBe(
      Math.round(backdropBox?.y ?? 0),
    );
    expect(Math.round(paneBackdropBox?.height ?? 0)).toBe(52);
    expect(paneBackdropBackground).not.toBe("rgba(0, 0, 0, 0)");
    expect(paneBackdropFilter).not.toBe("none");
    expect(Math.round(backdropBox?.x ?? 0)).toBe(Math.round(hostBox?.x ?? 0));
    expect(Math.round(backdropBox?.width ?? 0)).toBe(
      Math.round(hostBox?.width ?? 0),
    );
    expect(backdropFilter).not.toBe("none");
    expect(headerZIndex).toBeGreaterThan(backdropZIndex);
    expect(resizeHandleZIndex).toBeGreaterThan(backdropZIndex);
    expect(auxiliaryPaneAnimationName).toBe("none");

    await waitForAnimations(page);
  });
});
