import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const HELP_SEEN_KEY = "buzz.machine-onboarding.identity-key-help-seen.v1";

test("identity key help explains the first-run choice", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  const trigger = page.getByTestId("identity-key-help-trigger");
  // No initial opacity-0 assertion: on a slow runner the 2s reveal timer can
  // fire before the first assertion runs, failing the test for the wrong
  // reason. The reveal + persistence assertions below carry the coverage.
  await expect(trigger).toHaveCSS("opacity", "1", { timeout: 5000 });
  await expect
    .poll(() =>
      page.evaluate((key) => localStorage.getItem(key), HELP_SEEN_KEY),
    )
    .toBe("true");

  await trigger.click();

  const dialog = page.getByTestId("identity-key-help-dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "What’s an identity key?" }),
  ).toBeVisible();
  await expect(dialog).toHaveClass(/shadow-none/);
  await expect(page.getByTestId("dialog-overlay")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );

  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toHaveCSS("opacity", "1");

  await page.reload();
  await expect(page.getByTestId("identity-key-help-trigger")).toHaveCSS(
    "opacity",
    "1",
  );
});
