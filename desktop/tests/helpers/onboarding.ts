import type { Page } from "@playwright/test";

import { E2E_APP_ORIGIN } from "./bootstrap";
import { expect } from "./test";

export const E2E_IDENTITY_OVERRIDE_STORAGE_KEY =
  "buzz:e2e-identity-override.v1";

export async function seedActiveIdentity(
  page: Page,
  identity: { privateKey: string; pubkey: string; username: string },
) {
  await page.addInitScript(
    ({ expectedOrigin, identity: nextIdentity, storageKey }) => {
      if (location.origin !== expectedOrigin) return;
      window.localStorage.setItem(storageKey, JSON.stringify(nextIdentity));
    },
    {
      expectedOrigin: E2E_APP_ORIGIN,
      identity,
      storageKey: E2E_IDENTITY_OVERRIDE_STORAGE_KEY,
    },
  );
}

/** Continue past the created-key page without opening optional backup options. */
export async function passThroughBackupStep(page: Page) {
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("onboarding-next").click();
}
