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

async function openCreateBackup(page: Page) {
  await page.getByTestId("profile-encrypted-backup-row-toggle").click();
  const dialog = page.getByTestId("encrypted-backup-dialog");
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

test("identity settings expose independent create and test backup tools", async ({
  page,
}) => {
  await openBackupSettings(page);

  const createRow = page.getByTestId("profile-encrypted-backup-row");
  const testRow = page.getByTestId("profile-backup-test-row");
  await expect(createRow).toContainText("Create a key backup");
  await expect(createRow).toContainText("password-protected copy");
  await expect(testRow).toContainText("Test a key backup");
  await expect(testRow).toContainText("which identity it unlocks");

  const createDialog = await openCreateBackup(page);
  await expect(createDialog.getByLabel("Encryption password")).toBeVisible();
  await expect(testRow.getByText("Select your backup file")).toHaveCount(0);
  await createDialog.getByRole("button", { name: "Close" }).click();

  await testRow.getByTestId("profile-backup-test-row-toggle").click();
  await expect(testRow.getByText("Select your backup file")).toBeVisible();

  const reopenedDialog = await openCreateBackup(page);
  await expect(reopenedDialog.getByLabel("Encryption password")).toBeVisible();
  await expect(testRow.getByText("Select your backup file")).toBeVisible();
  await reopenedDialog.getByRole("button", { name: "Close" }).click();
});

test("creation requires a sufficiently long password and supports another download", async ({
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
  await expect(dialog.getByTestId("backup-passphrase-issue")).toHaveCount(0);
  await expect(submit).toBeDisabled();

  await password.fill("custom password");
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(dialog.getByTestId("backup-passphrase-input")).toHaveCount(0);
  await expect.poll(() => backupSaveCallCount(page)).toBe(1);
  await expect(submit).toHaveText("Download backup again");

  await submit.click();
  await expect.poll(() => backupSaveCallCount(page)).toBe(2);
});

test("submit replaces the form with progress and automatically opens save", async ({
  page,
}) => {
  await openBackupSettings(page, {
    backupEncryptionDelayMs: 750,
    backupSavePaths: [null, "/Users/test/Downloads/identity-retry.ncryptsec"],
  });
  const dialog = await openCreateBackup(page);
  await dialog.getByTestId("backup-passphrase-input").fill("progress password");
  await dialog.getByTestId("encrypted-backup-create").click();

  const progress = dialog.getByTestId("encrypted-backup-progress");
  await expect(progress).toBeVisible();
  await expect(progress).toHaveAttribute("aria-valuemax", "100");
  await expect(dialog.getByTestId("backup-passphrase-input")).toHaveCount(0);
  await expect(dialog.getByTestId("encrypted-backup-create")).toHaveCount(0);

  const downloadAgain = dialog.getByTestId("encrypted-backup-create");
  await expect(downloadAgain).toHaveText("Download backup again");
  await expect(progress).toHaveCount(0);
  await expect(dialog.getByTestId("backup-passphrase-input")).toHaveCount(0);
  await expect.poll(() => backupSaveCallCount(page)).toBe(1);

  await downloadAgain.click();
  await expect.poll(() => backupSaveCallCount(page)).toBe(2);
});

test("closing and reopening creation clears unsaved and completed state", async ({
  page,
}) => {
  await openBackupSettings(page);
  const dialog = await openCreateBackup(page);
  await dialog.getByTestId("backup-passphrase-input").fill("unsaved password");
  await dialog.getByRole("button", { name: "Close" }).click();

  await openCreateBackup(page);
  await expect(dialog.getByTestId("backup-passphrase-input")).toHaveValue("");

  await dialog
    .getByTestId("backup-passphrase-input")
    .fill("completed password");
  await dialog.getByTestId("encrypted-backup-create").click();
  await expect(dialog.getByTestId("encrypted-backup-create")).toHaveText(
    "Download backup again",
  );
  await dialog.getByRole("button", { name: "Close" }).click();

  await openCreateBackup(page);
  await expect(dialog.getByTestId("backup-passphrase-input")).toHaveValue("");
  await expect(dialog.getByTestId("encrypted-backup-create")).toHaveText(
    "Backup key",
  );
});

test("wrong backup password clears the attempt and permits a successful retry", async ({
  page,
}) => {
  await openBackupSettings(page, {
    backupVerificationErrors: ["Wrong password.", null],
  });
  const row = page.getByTestId("profile-backup-test-row");
  await row.getByTestId("profile-backup-test-row-toggle").click();
  await selectBackupFile(page);

  await verifyBackup(page, "wrong password");
  await expect(page.getByTestId("backup-test-error")).toHaveText(
    "Wrong password.",
  );
  await expect(page.getByTestId("backup-test-password")).toHaveValue("");
  await expect(page.getByTestId("backup-test-verify")).toBeDisabled();

  await verifyBackup(page, "correct password");
  await expect(page.getByTestId("backup-test-success")).toContainText(
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
  test(`successful verification identifies the ${identity.label} identity using only its npub`, async ({
    page,
  }) => {
    await openBackupSettings(page, {
      backupVerificationPubkeys: [identity.pubkey],
    });
    await page.getByTestId("profile-backup-test-row-toggle").click();
    await selectBackupFile(page);
    await verifyBackup(page, "one-time password");

    const success = page.getByTestId("backup-test-success");
    await expect(success).toContainText(identity.message);
    await expect(success.getByTestId("backup-test-npub")).toContainText(
      npubEncode(identity.pubkey),
    );
    await expect(success).not.toContainText(identity.pubkey);
    await expect(success).not.toContainText("one-time password");
    await expect(success).not.toContainText(BACKUP_FILE.buffer.toString());
  });
}
