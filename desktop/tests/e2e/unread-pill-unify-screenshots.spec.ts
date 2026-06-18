import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/unread-pill-unify";

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

const UNREAD_OFFSET_SECONDS = 60;

function unreadTimestamp() {
  return Math.floor(Date.now() / 1000) + UNREAD_OFFSET_SECONDS;
}

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

async function openGeneralWithUnreads(
  page: import("@playwright/test").Page,
  count: number,
) {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await emitUnreadMessages(page, count);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test.describe("unified unread pill — message area", () => {
  test("message-area-top-pill", async ({ page }) => {
    await openGeneralWithUnreads(page, 20);

    // Scroll up so the "N new messages" jump-to-oldest pill stays on screen.
    await page.getByTestId("message-timeline").evaluate((el) => {
      el.scrollTop = Math.floor(el.scrollHeight * 0.35);
    });
    await page.waitForTimeout(300);

    const pill = page.getByTestId("message-unread-pill");
    await expect(pill).toBeVisible();
    await expect(pill).toContainText("20 new messages");

    await page.screenshot({ path: `${SHOTS}/message-area-top-pill.png` });
  });

  test("message-area-bottom-pill", async ({ page }) => {
    await openGeneralWithUnreads(page, 60);

    // Let the channel settle (it auto-pins to the bottom on open), then scroll
    // up so plenty of content sits below the fold — the condition for the
    // bottom "jump to latest" pill to render.
    await page.waitForTimeout(500);
    await page.getByTestId("message-timeline").evaluate((el) => {
      el.scrollTop = Math.floor(el.scrollHeight * 0.25);
      el.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(500);

    const pill = page.getByTestId("message-scroll-to-latest");
    await expect(pill).toBeVisible();

    await page.screenshot({ path: `${SHOTS}/message-area-bottom-pill.png` });
  });
});

test.describe("unified unread pill — sidebar", () => {
  test("sidebar-more-unread-pills", async ({ page }) => {
    // A short viewport forces the channel list to overflow the sidebar scroll
    // area so unread rows can sit above and below the fold.
    await page.setViewportSize({ width: 1280, height: 460 });
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Mark several inactive channels unread (above + below "general" in the
    // list) so both the top and bottom "more unread" pills can appear.
    for (const ch of ["agents", "all-replies", "engineering", "random"]) {
      await waitForMockLiveSubscription(page, ch).catch(() => {});
      await page.getByTestId(`channel-${ch}`).click();
      await expect(page.getByTestId("chat-title")).toHaveText(ch);
      await emitMockMessage(
        page,
        ch,
        `Unread in ${ch}`,
        unreadTimestamp(),
      );
    }

    // Return to general so the others are inactive and show unread state.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Scroll the sidebar to the middle so unread rows fall off both ends.
    await page.waitForTimeout(400);
    await page
      .locator('[data-sidebar="content"]')
      .first()
      .evaluate((el) => {
        el.scrollTop = Math.floor(el.scrollHeight * 0.4);
        el.dispatchEvent(new Event("scroll"));
      });
    await page.waitForTimeout(500);

    await page.screenshot({ path: `${SHOTS}/sidebar-more-unread-pills.png` });

    // Scroll back to the top so the unread rows below the fold drive the
    // bottom "more unread" pill — capture it in the same unified style.
    await page
      .locator('[data-sidebar="content"]')
      .first()
      .evaluate((el) => {
        el.scrollTop = 0;
        el.dispatchEvent(new Event("scroll"));
      });
    await page.waitForTimeout(500);

    const bottomPill = page.getByTestId("sidebar-more-unread-below");
    await expect(bottomPill).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/sidebar-more-unread-bottom-pill.png`,
    });
  });
});
