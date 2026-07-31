import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/goose-update";

async function openGooseRuntimeRow(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");
  const row = page.getByTestId("doctor-runtime-goose");
  await expect(row).toBeVisible();
  await expect(page.getByTestId("doctor-runtime-ready-goose")).toBeVisible();
  return row;
}

test("latest Goose shows Ready without an Update action", async ({ page }) => {
  await installMockBridge(page, {
    gooseUpdateStatuses: [
      {
        status: "up_to_date",
        installed_version: "1.45.0",
        latest_version: "1.45.0",
      },
    ],
  });

  const row = await openGooseRuntimeRow(page);
  await expect(page.getByTestId("doctor-runtime-install-goose")).toHaveCount(0);
  await waitForAnimations(page);
  await row.screenshot({ path: `${SHOTS}/goose-current.png` });
});

test("older Goose shows Update and rechecks after success", async ({
  page,
}) => {
  await installMockBridge(page, {
    gooseUpdateStatuses: [
      {
        status: "update_available",
        installed_version: "1.44.0",
        latest_version: "1.45.0",
      },
      {
        status: "up_to_date",
        installed_version: "1.45.0",
        latest_version: "1.45.0",
      },
    ],
  });

  const row = await openGooseRuntimeRow(page);
  const update = page.getByTestId("doctor-runtime-install-goose");
  await expect(update).toHaveText("Update");
  await waitForAnimations(page);
  await row.screenshot({ path: `${SHOTS}/goose-update-available.png` });

  await update.click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMANDS__?.filter(
            (command) => command === "check_goose_update_status",
          ).length ?? 0,
      ),
    )
    .toBe(2);
  await expect(update).toHaveCount(0);
  await expect(page.getByTestId("doctor-runtime-ready-goose")).toBeVisible();
});
