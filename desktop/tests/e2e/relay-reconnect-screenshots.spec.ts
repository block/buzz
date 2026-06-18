import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/relay-reconnect-screenshots";

async function settle(page: import("@playwright/test").Page) {
  // Tolerate cancelled animations (skeleton → live swap rejects `.finished`
  // with AbortError) AND indefinitely-running ones (the degraded-state pulse
  // never resolves `.finished`): allSettled handles rejection, the timeout
  // race handles infinite animations so this can never hang the test.
  await page.evaluate(() =>
    Promise.race([
      Promise.allSettled(document.getAnimations().map((a) => a.finished)),
      new Promise((resolve) => setTimeout(resolve, 1_000)),
    ]),
  );
}

/** Drive the relay client into a state via the real E2E connection-state seam. */
async function driveConnectionState(
  page: import("@playwright/test").Page,
  state: "connected" | "reconnecting" | "stalled" | "disconnected",
) {
  await page.evaluate((s) => {
    const setter = (
      window as Window & {
        __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (state: string) => void;
      }
    ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__;
    if (!setter) throw new Error("E2E relay state setter not installed.");
    setter(s);
  }, state);
}

async function scrollSidebarToBottom(page: import("@playwright/test").Page) {
  // The relay block anchors at the bottom of the sidebar's scroll region and is
  // painted UNDER the absolute, z-30 profile footer (it sits within the footer's
  // 68px band when unscrolled — toBeVisible passes since it's in-DOM, but a human
  // can't see it). The scroll region is specifically [data-sidebar="content"];
  // scrolling it fully down lifts the block ~96px clear of the footer. Targeting
  // any old overflowing descendant matches the wrong element (e.g. a menu button),
  // so the selector must be exact.
  await page
    .getByTestId("app-sidebar")
    .locator('[data-sidebar="content"]')
    .evaluate((scroller) => {
      scroller.scrollTop = scroller.scrollHeight;
    });
}

test.describe("relay reconnect affordance screenshots", () => {
  test("01 — sidebar has no reconnect prompt when healthy", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
    // The relay block renders at the BOTTOM of the scrollable sidebar content,
    // below the fold at this viewport. Scroll to the bottom so 01 frames the
    // same region where 02 will show the block — making the absence legible.
    await scrollSidebarToBottom(page);
    await settle(page);

    // Frame the left sidebar directly — the relay-unreachable block lives there,
    // and a full-window shot makes its presence/absence illegible against the
    // unrelated top connection banner.
    await page.getByTestId("app-sidebar").screenshot({
      path: `${SHOTS}/01-sidebar-healthy.png`,
    });
  });

  test("02 — sidebar reconnect prompt shown when degraded, channels visible", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionState(page, "stalled");
    await expect(page.getByTestId("sidebar-relay-unreachable")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
    // The cached channel list stays visible alongside the prompt.
    await expect(page.getByTestId("channel-general")).toBeVisible();
    // Scroll the block clear of the occluding footer (symmetric with 01).
    await scrollSidebarToBottom(page);
    await settle(page);

    await page.getByTestId("app-sidebar").screenshot({
      path: `${SHOTS}/02-sidebar-degraded.png`,
    });
  });
});
