import { expect, test, type Page } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const CURRENT_PUBKEY = "deadbeef".repeat(8);
const DIFFERENT_PUBKEY = "c0ffee00".repeat(8);
const BACKUP_FILE = {
  name: "identity.ncryptsec",
  mimeType: "text/plain",
  buffer: Buffer.from("ncryptsec1mockbackupmaterial"),
};

async function openIdentity(page: Page) {
  const identity = page.getByTestId("profile-identity-card");
  if (
    !(await identity.evaluate(
      (element) => element instanceof HTMLDetailsElement && element.open,
    ))
  ) {
    await page.getByTestId("profile-identity-toggle").click();
  }
}

async function openBackupSettings(
  page: Page,
  mock?: Parameters<typeof installMockBridge>[1],
) {
  await installMockBridge(page, mock);
  await page.goto("/");
  await openSettings(page, "profile");
  await openIdentity(page);
}

async function openPrivateKeyMenu(page: Page) {
  const reveal = page.getByTestId("profile-private-key-toggle");
  if ((await reveal.textContent())?.trim() === "Reveal") {
    await reveal.click();
  }
  await page.getByTestId("nsec-actions").click();
  await expect(page.getByTestId("private-key-create-backup")).toBeVisible();
}

async function openCreateBackup(page: Page) {
  await openPrivateKeyMenu(page);
  await page.getByTestId("private-key-create-backup").click();
  const dialog = page.getByTestId("encrypted-backup-dialog");
  await expect(dialog).toBeVisible();
  return dialog;
}

async function openTestBackup(page: Page) {
  await openPrivateKeyMenu(page);
  await page.getByTestId("private-key-test-backup").click();
  const dialog = page.getByTestId("backup-test-dialog");
  await expect(dialog).toBeVisible();
  return dialog;
}

async function selectBackupFile(page: Page) {
  await page.getByTestId("backup-test-file-input").setInputFiles(BACKUP_FILE);
  await expect(page.getByTestId("backup-test-file-accepted")).toContainText(
    BACKUP_FILE.name,
  );
}

async function verifyBackup(page: Page, password: string) {
  await page.getByTestId("backup-test-password").fill(password);
  await page.getByTestId("backup-test-verify").click();
}

async function backupSaveCallCount(page: Page) {
  return page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "save_ncryptsec_copy",
      ).length ?? 0,
  );
}

test("private key menu replaces the backup settings rows", async ({ page }) => {
  await openBackupSettings(page);

  await expect(page.getByTestId("profile-encrypted-backup-row")).toHaveCount(0);
  await expect(page.getByTestId("profile-backup-test-row")).toHaveCount(0);

  await openPrivateKeyMenu(page);
  await expect(page.getByTestId("nsec-copy")).toContainText("Copy");
  await expect(page.getByTestId("private-key-create-backup")).toHaveText(
    "Create backup",
  );
  await expect(page.getByTestId("private-key-test-backup")).toHaveText(
    "Test backup",
  );

  await page.getByTestId("nsec-copy").click();
  await expect(page.getByText(/clipboard$/i)).toBeVisible();
  await expect(page.getByTestId("private-key-create-backup")).toHaveCount(0);

  await openPrivateKeyMenu(page);
  await page.getByTestId("private-key-test-backup").click();
  const testDialog = page.getByTestId("backup-test-dialog");
  await expect(testDialog).toContainText("Test a key backup");
  await expect(testDialog.getByText("Select your backup file")).toBeVisible();
  await expect(testDialog).toContainText("standard NIP-49 format");
});

test("creation requires a sufficiently long password and exposes a temporary header download", async ({
  page,
}) => {
  await openBackupSettings(page, {
    backupSavePaths: [
      "/Users/test/Downloads/identity.ncryptsec",
      "/Users/test/Desktop/identity-copy.ncryptsec",
    ],
  });
  const dialog = await openCreateBackup(page);

  const password = dialog.getByTestId("backup-passphrase-input");
  const submit = dialog.getByTestId("encrypted-backup-create");
  await expect(password).toHaveAttribute(
    "placeholder",
    "Password (min 12 characters)",
  );
  await expect(submit).toBeDisabled();
  await password.fill("short");
  await expect(submit).toBeDisabled();

  await password.fill("custom password");
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect.poll(() => backupSaveCallCount(page)).toBe(1);
  await expect(dialog).toBeHidden();

  const keyRow = page.getByTestId("profile-private-key-row");
  const download = keyRow.getByTestId("encrypted-backup-download");
  await expect(download).toBeVisible();
  await expect(download).toHaveText("Download backup");
  await expect(download).toHaveClass(/bg-primary/);
  await expect(
    download.getByTestId("encrypted-backup-availability-fill"),
  ).toBeVisible();
  await expect(keyRow.getByTestId("profile-private-key-toggle")).toBeVisible();

  await download.click();
  await expect.poll(() => backupSaveCallCount(page)).toBe(2);
});

test("encryption and native save continue after closing the dialog and settings", async ({
  page,
}) => {
  await openBackupSettings(page, {
    backupEncryptionDelayMs: 750,
    backupSavePaths: [null],
  });
  const dialog = await openCreateBackup(page);
  await dialog
    .getByTestId("backup-passphrase-input")
    .fill("background password");
  await dialog.getByTestId("encrypted-backup-create").click();
  await expect(dialog.getByTestId("encrypted-backup-progress")).toBeVisible();

  await dialog.getByRole("button", { name: "Close" }).click();
  await page.getByTestId("settings-back-to-app").click();
  await expect(page.getByTestId("settings-back-to-app")).toHaveCount(0);
  await expect(
    page.getByText("Preparing backup…", { exact: true }),
  ).toBeVisible();

  await expect.poll(() => backupSaveCallCount(page)).toBe(1);
  const readyToast = page.getByText("Backup ready to download", {
    exact: true,
  });
  await expect(readyToast).toBeVisible();
  await expect(
    page.getByText("Your backup will be available to download for 5 minutes.", {
      exact: true,
    }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByTestId("settings-back-to-app")).toBeVisible();
  await openIdentity(page);
  await expect(page.getByTestId("encrypted-backup-download")).toBeVisible();
});

test("the temporary download expires after five minutes", async ({ page }) => {
  await page.clock.install({ time: new Date("2026-07-29T12:00:00Z") });
  await openBackupSettings(page);
  const dialog = await openCreateBackup(page);
  await dialog.getByTestId("backup-passphrase-input").fill("expiring password");
  await dialog.getByTestId("encrypted-backup-create").click();
  await page.clock.fastForward(1);

  await expect.poll(() => backupSaveCallCount(page)).toBe(1);
  const download = page.getByTestId("encrypted-backup-download");
  await expect(download).toHaveText("Download backup");
  await expect(
    download.getByTestId("encrypted-backup-availability-fill"),
  ).toBeVisible();
  await page.clock.fastForward(5 * 60 * 1000 + 1);
  await expect(page.getByTestId("encrypted-backup-download")).toHaveCount(0);
});

test("wrong backup password permits a successful retry in the test modal", async ({
  page,
}) => {
  await openBackupSettings(page, {
    backupVerificationErrors: ["Wrong password.", null],
  });
  const dialog = await openTestBackup(page);
  await selectBackupFile(page);

  await verifyBackup(page, "wrong password");
  await expect(dialog.getByTestId("backup-test-error")).toHaveText(
    "Wrong password.",
  );
  await expect(dialog.getByTestId("backup-test-password")).toHaveValue("");
  await expect(dialog.getByTestId("backup-test-verify")).toBeDisabled();

  await verifyBackup(page, "correct password");
  await expect(dialog.getByTestId("backup-test-success")).toContainText(
    "It restores your current Buzz identity.",
  );
});

for (const identity of [
  {
    label: "current",
    pubkey: CURRENT_PUBKEY,
    message: "It restores your current Buzz identity.",
  },
  {
    label: "different",
    pubkey: DIFFERENT_PUBKEY,
    message: "It restores a different identity than the one signed in here.",
  },
]) {
  test(`successful modal verification identifies the ${identity.label} identity using only its npub`, async ({
    page,
  }) => {
    await openBackupSettings(page, {
      backupVerificationPubkeys: [identity.pubkey],
    });
    const dialog = await openTestBackup(page);
    await selectBackupFile(page);
    await verifyBackup(page, "one-time password");

    const success = dialog.getByTestId("backup-test-success");
    await expect(success).toContainText(identity.message);
    await expect(success.getByTestId("backup-test-npub")).toContainText(
      npubEncode(identity.pubkey),
    );
    await expect(success).not.toContainText(identity.pubkey);
    await expect(success).not.toContainText("one-time password");
    await expect(success).not.toContainText(BACKUP_FILE.buffer.toString());
  });
}
