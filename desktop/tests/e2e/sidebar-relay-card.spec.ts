import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const BLOCK_RELAY_URL = "wss://sprout-oss.stage.blox.sqprod.co";
const CUSTOM_RELAY_URL = "wss://relay.example.com";
const CONNECT_ERROR = "relay unreachable: could not connect to relay";
const PROXY_ERROR =
  "relay unreachable: relay returned an unexpected HTML page (VPN or proxy sign-in?)";
const CLOUDFLARE_ACCESS_ERROR =
  "relay unreachable: 403 Forbidden from Cloudflare Access";
const CLOUDFLARE_ACCESS_REDIRECT_ERROR =
  "relay unreachable: network sign-in required (Cloudflare Access / VPN) - re-authenticate and reconnect";
const RELAY_AUTH_ERROR = "Relay authentication rejected.";

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

test("Block workspace sidebar generic relay failures offer the VPN card", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: CONNECT_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-vpn-off");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");
  await expect(page.getByTestId("sidebar-connect-vpn")).toBeVisible();
  await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
});

test("Block workspace sidebar proxy failures offer the VPN card", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: PROXY_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-vpn-off");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");
  await expect(page.getByTestId("sidebar-connect-vpn")).toBeVisible();
  await expect(page.getByTestId("sidebar-vpn-access-refresh")).toHaveCount(0);
});

test("Block workspace sidebar Cloudflare Access failures offer access refresh", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: CLOUDFLARE_ACCESS_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-vpn-access-refresh");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Refresh VPN access");
  await expect(page.getByTestId("sidebar-refresh-vpn-access")).toBeVisible();
  await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
});

test("Block workspace sidebar Cloudflare Access redirects offer the VPN card", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: CLOUDFLARE_ACCESS_REDIRECT_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-vpn-off");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");
  await expect(page.getByTestId("sidebar-connect-vpn")).toBeVisible();
  await expect(page.getByTestId("sidebar-vpn-access-refresh")).toHaveCount(0);
});

test("Block workspace sidebar application auth failures stay on the error path", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: RELAY_AUTH_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  await expect(page.getByText(RELAY_AUTH_ERROR)).toBeVisible();
  await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
  await expect(page.getByTestId("sidebar-vpn-access-refresh")).toHaveCount(0);
  await expect(page.getByTestId("sidebar-vpn-off")).toHaveCount(0);
});

test("Block workspace sidebar VPN action shows connected before hiding", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: CONNECT_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-vpn-off");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");

  await setChannelsReadError(page, null);
  await page.getByTestId("sidebar-connect-vpn").click();

  await expect(card).toContainText("Connected");
  await expect(card).not.toContainText("Click to connect");

  await page.waitForTimeout(3_000);
  await expect(card).toContainText("Connected");
  await expect(card).toBeHidden({ timeout: 5_000 });
});

test("Block workspace sidebar VPN action stays actionable when refresh still fails", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: CONNECT_ERROR },
    { relayWsUrl: BLOCK_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-vpn-off");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");

  await page.getByTestId("sidebar-connect-vpn").click();

  await page.waitForTimeout(500);
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");
  await expect(card).not.toContainText("Connected");

  await page.waitForTimeout(6_500);
  await expect(card).toBeVisible();
  await expect(card).toContainText("Turn on VPN");
  await expect(card).not.toContainText("Connected");
});

test("custom workspace sidebar proxy failures stay generic", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { channelsReadError: PROXY_ERROR },
    { relayWsUrl: CUSTOM_RELAY_URL },
  );

  await page.goto("/");

  const card = page.getByTestId("sidebar-relay-unreachable");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Can't reach the relay");
  await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
  await expect(page.getByTestId("sidebar-vpn-access-refresh")).toHaveCount(0);
  await expect(page.getByTestId("sidebar-vpn-off")).toHaveCount(0);
});
