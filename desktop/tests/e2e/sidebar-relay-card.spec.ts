import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const CONNECT_ERROR = "relay unreachable: could not connect to relay";
const PROXY_ERROR =
  "relay unreachable: relay returned an unexpected HTML page (network sign-in?)";
const ACCESS_ERROR = "relay unreachable: 403 Forbidden";
const RELAY_AUTH_ERROR = "Relay authentication rejected.";

type RelayConnectionState =
  | "connected"
  | "connecting"
  | "disconnected"
  | "idle"
  | "reconnecting"
  | "stalled";

async function setChannelsReadError(page: Page, error: string | null) {
  await page.evaluate((nextError) => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { channelsReadError?: string } };
    };

    if (!testWindow.__BUZZ_E2E__?.mock) {
      throw new Error("Mock bridge config is not installed.");
    }

    if (nextError === null) {
      delete testWindow.__BUZZ_E2E__.mock.channelsReadError;
      return;
    }

    testWindow.__BUZZ_E2E__.mock.channelsReadError = nextError;
  }, error);
}

async function setRelayConnectionState(
  page: Page,
  state: RelayConnectionState,
) {
  await page.waitForFunction(
    () =>
      typeof (
        window as Window & {
          __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: unknown;
        }
      ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ === "function",
  );
  await page.evaluate((nextState) => {
    const testWindow = window as Window & {
      __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (
        state: RelayConnectionState,
      ) => void;
    };

    const setConnectionState =
      testWindow.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__;
    if (!setConnectionState) {
      throw new Error("Mock relay connection state helper is not installed.");
    }

    setConnectionState(nextState);
  }, state);
}

async function expectGenericReconnectCard(page: Page) {
  const card = page.getByTestId("sidebar-relay-unreachable");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Can't reach the relay");
  await expect(card).toContainText("Click to connect");
  await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
  return card;
}

test("sidebar generic relay failures use the reconnect card", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: CONNECT_ERROR });

  await page.goto("/");

  await expectGenericReconnectCard(page);
});

test("sidebar proxy sign-in failures use the reconnect card", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: PROXY_ERROR });

  await page.goto("/");

  await expectGenericReconnectCard(page);
});

test("sidebar access failures use the reconnect card", async ({ page }) => {
  await installMockBridge(page, { channelsReadError: ACCESS_ERROR });

  await page.goto("/");

  await expectGenericReconnectCard(page);
});

test("collapsed sidebar relay failures use the connection banner", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: CONNECT_ERROR });

  await page.goto("/");

  await expectGenericReconnectCard(page);

  await page
    .getByRole("button", { exact: true, name: "Toggle Sidebar" })
    .click();
  await expect(
    page.locator('[data-state="collapsed"][data-collapsible="offcanvas"]'),
  ).toHaveCount(1);

  const banner = page.getByTestId("connection-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Can't reach the relay.");

  await setChannelsReadError(page, null);
  await page.getByTestId("connection-banner-reconnect").click();
  await expect(banner).toBeHidden({ timeout: 10_000 });
});

test("sidebar stalled relay state uses the reconnect card", async ({
  page,
}) => {
  await installMockBridge(page);

  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await setRelayConnectionState(page, "stalled");

  await expectGenericReconnectCard(page);
});

test("sidebar application auth disconnects stay on the error path", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: RELAY_AUTH_ERROR });

  await page.goto("/");
  await setRelayConnectionState(page, "disconnected");

  await expect(page.getByText(RELAY_AUTH_ERROR)).toBeVisible();
  await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
});

test("sidebar reconnect action shows connected before hiding", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: CONNECT_ERROR });

  await page.goto("/");

  const card = await expectGenericReconnectCard(page);

  await setChannelsReadError(page, null);
  await page.getByTestId("sidebar-reconnect").click();

  await expect(card).toContainText("Connected");
  await expect(card).not.toContainText("Click to connect");

  await page.waitForTimeout(3_000);
  await expect(card).toContainText("Connected");
  await expect(card).toBeHidden({ timeout: 5_000 });
});

test("sidebar reconnect action suppresses stale refresh errors after success", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: CONNECT_ERROR });

  await page.goto("/");

  const card = await expectGenericReconnectCard(page);

  await page.getByTestId("sidebar-reconnect").click();

  await page.waitForTimeout(500);
  await expect(card).toBeVisible();
  await expect(card).toContainText("Connected");
  await expect(card).not.toContainText("Can't reach the relay");

  await page.waitForTimeout(6_500);
  await expect(card).toBeHidden();
});

test("sidebar connected success clears when relay degrades again", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadError: CONNECT_ERROR });

  await page.goto("/");

  const card = await expectGenericReconnectCard(page);

  await setChannelsReadError(page, null);
  await page.getByTestId("sidebar-reconnect").click();

  await expect(card).toContainText("Connected");

  await setRelayConnectionState(page, "stalled");

  await expect(card).toContainText("Can't reach the relay", {
    timeout: 10_000,
  });
  await expect(card).toContainText("Click to connect");
  await expect(card).not.toContainText("Connected");
});
