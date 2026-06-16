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
  state: "connected" | "disconnected",
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

async function openProfilePopover(page: import("@playwright/test").Page) {
  await page.getByTestId("sidebar-profile-avatar-button").click();
  // Anchor on a stable popover child so the screenshot captures the open menu.
  await expect(page.getByTestId("profile-popover-settings")).toBeVisible();
  // Await the Radix open animation on the [data-state] ancestor — the popper
  // wrapper carries no animations, so screenshots would otherwise capture a
  // half-faded popover.
  await page.getByTestId("profile-popover-settings").evaluate((el) =>
    Promise.all(
      el
        .closest("[data-state]")
        ?.getAnimations()
        .map((a) => a.finished) ?? [],
    ),
  );
}

test.describe("relay reconnect affordance screenshots", () => {
  test("01 — profile popover reconnect item hidden when healthy", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await openProfilePopover(page);
    await expect(page.getByTestId("profile-popover-reconnect")).toHaveCount(0);
    await settle(page);

    await page.screenshot({
      path: `${SHOTS}/01-profile-popover-healthy.png`,
    });
  });

  test("02 — profile popover reconnect item shown when degraded", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionState(page, "disconnected");
    await openProfilePopover(page);
    await expect(page.getByTestId("profile-popover-reconnect")).toBeVisible({
      timeout: 5_000,
    });
    await settle(page);

    await page.screenshot({
      path: `${SHOTS}/02-profile-popover-degraded.png`,
    });
  });

  test("03 — sidebar has no reconnect prompt when healthy", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
    await settle(page);

    await page.screenshot({ path: `${SHOTS}/03-sidebar-healthy.png` });
  });

  test("04 — sidebar reconnect prompt shown when degraded, channels visible", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionState(page, "disconnected");
    await expect(page.getByTestId("sidebar-relay-unreachable")).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
    // The cached channel list stays visible alongside the prompt.
    await expect(page.getByTestId("channel-general")).toBeVisible();
    await settle(page);

    await page.screenshot({ path: `${SHOTS}/04-sidebar-degraded.png` });
  });
});
