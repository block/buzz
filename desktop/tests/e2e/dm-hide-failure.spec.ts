import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const DM_NAME = "alice-tyler";
const RELAY_ERROR = "relay error 400: forbidden: not a member of this DM";

test("a rejected DM close reports the relay error and keeps the row", async ({
  page,
}) => {
  await page.addInitScript((message) => {
    window.__BUZZ_E2E_FAIL_HIDE_DM__ = message;
  }, RELAY_ERROR);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const dmRow = page.getByTestId(`channel-${DM_NAME}`);
  await expect(dmRow).toBeVisible();

  await dmRow.hover();
  await page.getByTestId(`hide-dm-${DM_NAME}`).click();

  await expect(
    page.locator("[data-sonner-toast]").filter({ hasText: RELAY_ERROR }),
  ).toBeVisible();

  // The optimistic removal must roll back: the conversation is still there,
  // so the sidebar must not claim it went away.
  await expect(dmRow).toBeVisible();
});
