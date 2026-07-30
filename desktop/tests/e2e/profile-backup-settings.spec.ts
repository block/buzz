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

test("creation requires a sufficiently long password and supports another download", async ({
  page,
}) => {
  await openBackupSettings(page, {
    backupSavePaths: [
      "/Users/test/Downloads/identity.ncryptsec",
      "/Users/test/Desktop/identity-copy.ncryptsec",
    ],
  });
  const row = page.getByTestId("profile-encrypted-backup-row");
  await row.getByTestId("profile-encrypted-backup-row-toggle").click();

  const password = row.getByTestId("backup-passphrase-input");
  const submit = row.getByTestId("encrypted-backup-create");
  await expect(password).toHaveAttribute(
    "placeholder",
    "Password (min 12 characters)",
  );
  await expect(submit).toBeDisabled();
  await password.fill("short");
  await expect(row.getByTestId("backup-passphrase-issue")).toHaveText(
    "Use at least 12 characters.",
  );
  await expect(submit).toBeDisabled();

  await password.fill("custom password");
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(row.getByTestId("backup-saved-password-mask")).toBeVisible();
  await expect(row.getByTestId("encrypted-backup-saved-path")).toContainText(
    "/Users/test/Downloads/identity.ncryptsec",
  );
  await expect(submit).toHaveText("Download backup again");

  await submit.click();
  await expect(row.getByTestId("encrypted-backup-saved-path")).toContainText(
    "/Users/test/Desktop/identity-copy.ncryptsec",
  );
});

test("closing and reopening creation clears unsaved and completed state", async ({
  page,
}) => {
  await openBackupSettings(page);
  const row = page.getByTestId("profile-encrypted-backup-row");
  const toggle = row.getByTestId("profile-encrypted-backup-row-toggle");
  await toggle.click();
  await row.getByTestId("backup-passphrase-input").fill("unsaved password");
  await toggle.click();
  await toggle.click();
  await expect(row.getByTestId("backup-passphrase-input")).toHaveValue("");

  await row.getByTestId("backup-passphrase-input").fill("completed password");
  await row.getByTestId("encrypted-backup-create").click();
  await expect(row.getByTestId("encrypted-backup-created")).toBeVisible();
  await toggle.click();
  await toggle.click();
  await expect(row.getByTestId("backup-passphrase-input")).toHaveValue("");
  await expect(row.getByTestId("encrypted-backup-created")).toHaveCount(0);
  await expect(row.getByTestId("encrypted-backup-create")).toHaveText(
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
