import type { Page } from "@playwright/test";
import { expect } from "@playwright/test";

export const E2E_IDENTITY_OVERRIDE_STORAGE_KEY =
  "buzz:e2e-identity-override.v1";

export async function seedActiveIdentity(
  page: Page,
  identity: { privateKey: string; pubkey: string; username: string },
) {
  await page.addInitScript(
    ({ identity: nextIdentity, storageKey }) => {
      window.localStorage.setItem(storageKey, JSON.stringify(nextIdentity));
    },
    { identity, storageKey: E2E_IDENTITY_OVERRIDE_STORAGE_KEY },
  );
}

/** Navigate through the backup steps (fresh-key path). */
export async function passThroughBackupStep(page: Page) {
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  // Next always leads into the "Backup your key" step; backing up is
  // recommended, never required — "Skip for now" there advances to setup.
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
  await page.getByTestId("onboarding-skip").click();
}
