import { expect, type Page } from "@playwright/test";

type SettingsSection =
  | "profile"
  | "notifications"
  | "agents"
  | "channel-templates"
  | "compute"
  | "appearance"
  | "shortcuts"
  | "tokens"
  | "community-members"
  | "mobile"
  | "updates";

export async function openProfileMenu(page: Page) {
  await page.getByTestId("open-settings").click();
  await expect(page.getByTestId("profile-popover")).toBeVisible();
}

export async function openSettings(page: Page, section?: SettingsSection) {
  await openProfileMenu(page);
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();

  if (section) {
    await page.getByTestId(`settings-nav-${section}`).click();
  }
}
