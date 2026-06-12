import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function setMockWebsocketSendsStalled(
  page: import("@playwright/test").Page,
  stall: boolean,
) {
  await page.evaluate((shouldStall) => {
    const setter = (
      window as Window & {
        __BUZZ_E2E_SET_STALL_WEBSOCKET_SENDS__?: (stall: boolean) => void;
      }
    ).__BUZZ_E2E_SET_STALL_WEBSOCKET_SENDS__;
    if (!setter) {
      throw new Error("E2E websocket stall setter is not installed.");
    }
    setter(shouldStall);
  }, stall);
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("passive relay watchdog does not write while the websocket is half-open", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );

  await setMockWebsocketSendsStalled(page, true);

  // Wait longer than the old active-probe interval. If the watchdog still
  // writes probes, the mocked plugin send would never resolve and mark the
  // mock plugin mutex as wedged. Future reconnects would then be unable to
  // register, matching the tauri-plugin-websocket failure mode. The passive
  // watchdog should perform no writes of its own during this window.
  await page.waitForTimeout(22_000);

  await setMockWebsocketSendsStalled(page, false);
  const message = `recovered after passive idle ${Date.now()}`;
  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);
});
