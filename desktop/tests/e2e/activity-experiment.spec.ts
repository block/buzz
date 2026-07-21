import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function expectHomeLabel(
  page: import("@playwright/test").Page,
  label: "Activity" | "Inbox",
) {
  await expect(
    page.getByTestId("sidebar-primary-menu").getByRole("button", {
      name: label,
      exact: true,
    }),
  ).toBeVisible();
}

async function openHomeFilters(page: import("@playwright/test").Page) {
  await page.getByTestId("inbox-filter-trigger").click();
}

async function openExperiments(page: import("@playwright/test").Page) {
  const settingsItem = page.getByTestId("profile-popover-settings");
  if (!(await settingsItem.isVisible())) {
    await page.getByTestId("open-settings").click();
  }
  await expect(settingsItem).toBeVisible();
  await settingsItem.dispatchEvent("click");
  await page.getByTestId("settings-nav-experimental").click();
}

test("Activity can be enabled and disabled as an experiment", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await expectHomeLabel(page, "Inbox");
  await openHomeFilters(page);
  await expect(
    page.getByRole("menuitemradio", { name: "Activity", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await openExperiments(page);
  const activityToggle = page.getByTestId("feature-toggle-activity");
  await expect(activityToggle).not.toBeChecked();
  await activityToggle.click();
  await expect(activityToggle).toBeChecked();
  await page.getByTestId("settings-back-to-app").click();

  await expectHomeLabel(page, "Activity");
  await openHomeFilters(page);
  await expect(
    page.getByRole("menuitemradio", { name: "Activity", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("menuitemradio", { name: "Agents", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await openExperiments(page);
  await page.getByTestId("feature-toggle-activity").click();
  await page.getByTestId("settings-back-to-app").click();
  await expectHomeLabel(page, "Inbox");
});
