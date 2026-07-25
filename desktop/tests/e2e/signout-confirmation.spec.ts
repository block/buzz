/**
 * E2E tests for the destructive sign-out confirmation flow.
 *
 * Signing out wipes the identity key and all local data, so the dialog gates
 * "Delete My Data" behind two explicit steps:
 *   1. backup — reveal/copy the nsec, then check "I have saved my private key"
 *   2. typed confirmation — type the exact phrase "wipe all my data"
 */
import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const CONFIRM_PHRASE = "wipe all my data";

// The mock bridge routes copy_text_to_clipboard through navigator.clipboard,
// which requires explicit permissions in headless Chromium.
test.use({ permissions: ["clipboard-read", "clipboard-write"] });

async function openSignOutDialog(page: Page) {
  await openSettings(page, "profile");
  const section = page.getByTestId("settings-signout");
  await section.scrollIntoViewIfNeeded();
  await page.getByTestId("signout-open-dialog").click();
  await expect(page.getByRole("alertdialog")).toBeVisible({ timeout: 5_000 });
}

test("delete button unlocks only after backup + typed phrase", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSignOutDialog(page);

  const deleteButton = page.getByTestId("signout-confirm");
  const backupCheckbox = page.getByTestId("signout-backup-confirm");
  const phraseInput = page.getByTestId("signout-confirm-phrase");

  // Everything locked initially: no key interaction yet.
  await expect(deleteButton).toBeDisabled();
  await expect(backupCheckbox).toBeDisabled();

  // Copying the key unlocks the backup checkbox.
  await page.getByTestId("nsec-copy").click();
  await expect(backupCheckbox).toBeEnabled();
  await backupCheckbox.click();

  // Backup alone is not enough.
  await expect(deleteButton).toBeDisabled();

  // Wrong phrase keeps it locked.
  await phraseInput.fill("wipe my data");
  await expect(deleteButton).toBeDisabled();

  // Exact phrase (case/whitespace tolerant) unlocks it.
  await phraseInput.fill(`  ${CONFIRM_PHRASE.toUpperCase()}  `);
  await expect(deleteButton).toBeEnabled();

  // Clearing the phrase locks it again.
  await phraseInput.fill("");
  await expect(deleteButton).toBeDisabled();
});

test("reveal also unlocks the backup checkbox", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSignOutDialog(page);

  const backupCheckbox = page.getByTestId("signout-backup-confirm");
  await expect(backupCheckbox).toBeDisabled();

  await page.getByTestId("nsec-reveal-toggle").click();
  await expect(backupCheckbox).toBeEnabled();
});

test("completing both gates invokes sign_out", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSignOutDialog(page);

  await page.getByTestId("nsec-copy").click();
  await page.getByTestId("signout-backup-confirm").click();
  await page.getByTestId("signout-confirm-phrase").fill(CONFIRM_PHRASE);

  const deleteButton = page.getByTestId("signout-confirm");
  await expect(deleteButton).toBeEnabled();
  await deleteButton.click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
          ).__BUZZ_E2E_COMMANDS__?.includes("sign_out") ?? false,
      ),
    )
    .toBe(true);
});

test("cancel resets the gates for the next open", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSignOutDialog(page);

  // Satisfy both gates, then cancel.
  await page.getByTestId("nsec-copy").click();
  await page.getByTestId("signout-backup-confirm").click();
  await page.getByTestId("signout-confirm-phrase").fill(CONFIRM_PHRASE);
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("alertdialog")).not.toBeVisible();

  // Reopen — everything must be locked again.
  await page.getByTestId("signout-open-dialog").click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await expect(page.getByTestId("signout-backup-confirm")).toBeDisabled();
  await expect(page.getByTestId("signout-confirm-phrase")).toHaveValue("");
  await expect(page.getByTestId("signout-confirm")).toBeDisabled();
});

test("nsec load failure still allows sign-out (backup step degrades)", async ({
  page,
}) => {
  await installMockBridge(page, { nsecError: "Keychain locked" });
  await page.goto("/");
  await openSignOutDialog(page);

  // Error shown in place of the key; checkbox is usable so the user is not
  // permanently locked out of signing out.
  await expect(page.getByTestId("signout-nsec-error")).toContainText(
    "Keychain locked",
  );
  const backupCheckbox = page.getByTestId("signout-backup-confirm");
  await expect(backupCheckbox).toBeEnabled();

  await backupCheckbox.click();
  await page.getByTestId("signout-confirm-phrase").fill(CONFIRM_PHRASE);
  await expect(page.getByTestId("signout-confirm")).toBeEnabled();
});
