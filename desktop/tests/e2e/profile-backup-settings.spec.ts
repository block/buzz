import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

async function openIdentity(page: import("@playwright/test").Page) {
  const identity = page.getByTestId("profile-identity-card");
  if (
    !(await identity.evaluate(
      (element) => element instanceof HTMLDetailsElement && element.open,
    ))
  ) {
    await page.getByTestId("profile-identity-toggle").click();
  }
}

test("identity settings expose independent create and test backup tools", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "profile");
  await openIdentity(page);

  const createRow = page.getByTestId("profile-encrypted-backup-row");
  const testRow = page.getByTestId("profile-backup-test-row");
  await expect(createRow).toContainText("Create a key backup");
  await expect(createRow).toContainText("password-protected copy");
  await expect(testRow).toContainText("Test a key backup");
  await expect(testRow).toContainText("which identity it unlocks");

  await createRow.getByTestId("profile-encrypted-backup-row-toggle").click();
  await expect(createRow.getByLabel("Encryption password")).toBeVisible();
  await expect(testRow.getByText("Select your backup file")).toHaveCount(0);

  await testRow.getByTestId("profile-backup-test-row-toggle").click();
  await expect(testRow.getByText("Select your backup file")).toBeVisible();
  await expect(createRow.getByLabel("Encryption password")).toBeVisible();

  await createRow.getByTestId("profile-encrypted-backup-row-toggle").click();
  await expect(createRow.getByLabel("Encryption password")).toHaveCount(0);
  await expect(testRow.getByText("Select your backup file")).toBeVisible();
});
