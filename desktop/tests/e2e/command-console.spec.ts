import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

type CommandConsoleE2eWindow = Window & {
  __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (state: string) => void;
};

test("Command Console opens from the sidebar with truthful local-first boundaries", async ({
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

  await expect(
    consoleScreen.getByRole("heading", {
      name: "Daily Command Brief",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    consoleScreen.getByText("No Daily Command Brief has been generated."),
  ).toBeVisible();
  await expect(consoleScreen.getByText("Not yet operational")).toHaveCount(0);

  await expect(consoleScreen.getByTestId("command-status-relay")).toContainText(
    "Unavailable",
  );
  await expect(
    consoleScreen.getByTestId("command-status-local-compute"),
  ).toContainText("Offline");
  await expect(
    consoleScreen.getByTestId("command-status-lm-studio"),
  ).toContainText("Unavailable");
  for (const service of ["memory", "rag", "apple-inputs"]) {
    await expect(
      consoleScreen.getByTestId(`command-status-${service}`),
    ).toContainText("Unavailable");
  }
  await expect(
    consoleScreen.getByText("Connected", { exact: true }),
  ).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: true,
    path: "test-results/command-console/phase-4-local-runtime.png",
  });
});
