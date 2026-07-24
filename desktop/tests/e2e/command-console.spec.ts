import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const ADVISERS = [
  "Chief of Staff",
  "Operations",
  "Navigation",
  "Daily Routine",
  "Reporting",
  "Plans",
] as const;

type CommandConsoleE2eWindow = Window & {
  __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (state: string) => void;
};

test("Command Console opens from the sidebar with truthful Phase 1 boundaries", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("open-command-console-view").click();

  await expect(page).toHaveURL(/#\/console$/);
  const consoleScreen = page.getByTestId("command-console-screen");
  await expect(consoleScreen).toBeVisible();

  await page.evaluate(() => {
    const setRelayState = (window as CommandConsoleE2eWindow)
      .__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__;
    if (!setRelayState) {
      throw new Error("E2E relay state setter is not installed.");
    }
    setRelayState("connecting");
  });

  await expect(
    consoleScreen.getByTestId("command-console-official-banner"),
  ).toContainText("OFFICIAL");

  for (const adviser of ADVISERS) {
    await expect(
      consoleScreen.getByText(adviser, { exact: true }),
    ).toBeVisible();
  }
  await expect(consoleScreen.getByText("Not yet operational")).toHaveCount(
    ADVISERS.length,
  );

  await expect(consoleScreen.getByTestId("command-status-relay")).toContainText(
    "Unavailable",
  );
  await expect(
    consoleScreen.getByTestId("command-status-local-compute"),
  ).toContainText("Offline");
  await expect(consoleScreen.getByText("Not configured")).toHaveCount(4);
  await expect(
    consoleScreen.getByText("Connected", { exact: true }),
  ).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: true,
    path: "test-results/command-console/phase-1-foundation.png",
  });
});
