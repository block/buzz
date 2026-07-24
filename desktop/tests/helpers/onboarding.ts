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

/**
 * From the landing screen, enter the advanced (no-email) identity screen —
 * the create-key / import-key choice behind "Sign up without email".
 */
export async function enterAdvancedIdentity(page: Page) {
  await page.getByRole("button", { name: "Sign up without email" }).click();
  await expect(
    page.getByRole("button", { name: "Create a new identity key" }),
  ).toBeVisible();
}

/** From the landing screen, create a fresh key via the advanced path. */
export async function createFreshIdentityKey(page: Page) {
  await enterAdvancedIdentity(page);
  await page.getByRole("button", { name: "Create a new identity key" }).click();
}

/** Navigate through the backup step (fresh-key path). */
export async function passThroughBackupStep(page: Page) {
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("nsec-value")).toBeVisible();
  await page.getByTestId("onboarding-next").click();
}
