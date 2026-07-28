import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const ENDPOINT = "https://api.worldmonitor.app/mcp";

test("World Monitor connects through OAuth without an API-key field", async ({
  page,
}) => {
  await installMockBridge(page, {
    worldMonitorConnection: {
      endpoint: ENDPOINT,
      status: "not_connected",
      briefUsed: 0,
      briefLimit: 25,
      directUsed: 0,
      directLimit: 25,
    },
  });
  await page.goto("/");
  await page.getByTestId("open-command-console-view").click();

  const card = page.getByTestId("world-monitor-connection");
  await expect(card.getByText("Not connected", { exact: true })).toBeVisible();
  await expect(card.getByText("Brief 0/25", { exact: true })).toBeVisible();
  await expect(
    card.getByText("Direct questions 0/25", { exact: true }),
  ).toBeVisible();

  await expect(card.getByLabel("World Monitor API key")).toHaveCount(0);
  await card
    .getByRole("button", { name: "Connect World Monitor", exact: true })
    .click();
  await expect(card.getByText("Connected", { exact: true })).toBeVisible();

  await card.getByRole("button", { name: "Test connection" }).click();
  await expect(card.getByText("Connected", { exact: true })).toBeVisible();

  await card.getByRole("button", { name: "Disconnect", exact: true }).click();
  await expect(card.getByText("Not connected", { exact: true })).toBeVisible();
  await expect(
    card.getByText("No API key is required.", { exact: false }),
  ).toBeVisible();
});
