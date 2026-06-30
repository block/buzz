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
  | "relay-members"
  | "mobile"
  | "updates"
  | "doctor";

export async function openProfileMenu(page: Page) {
  await page.getByTestId("open-settings").click();
  await expect(page.getByTestId("profile-popover")).toBeVisible();
}

export async function openSettings(page: Page, section?: SettingsSection) {
  await openProfileMenu(page);
  const settingsItem = page.getByTestId("profile-popover-settings");
  await expect(settingsItem).toBeVisible();
  await settingsItem.click({ force: true });
  await expect(page.getByTestId("settings-view")).toBeVisible();

  if (section) {
    await page.getByTestId(`settings-nav-${section}`).click();
  }
}
