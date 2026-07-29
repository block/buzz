/**
 * Compact E2E tests for NsecRevealRow in ProfileSettingsCard.
 * Covers: reveal fetches + renders masked value, error state, collapse clears state.
 */
import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

/** Expand the identity details section if it is not already open. */
async function expandIdentity(page: import("@playwright/test").Page) {
  const identity = page.getByTestId("profile-identity-card");
  const isOpen = await identity.evaluate(
    (el) => el instanceof HTMLDetailsElement && el.open,
  );
  if (!isOpen) {
    await page.getByTestId("profile-identity-toggle").click();
  }
  await expect(page.getByTestId("profile-identity-details")).toBeVisible();
}

test("reveal shows masked nsec value and hides it again on collapse", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "profile");
  await expandIdentity(page);

  const revealToggle = page.getByTestId("profile-private-key-toggle");
  await expect(revealToggle).toBeVisible();
  await expect(revealToggle).toHaveText("Reveal");

  // Reveal — the masked nsec display should appear.
  await revealToggle.click();
  await expect(revealToggle).toHaveText("Hide");
  const nsecDisplay = page.locator(
    '[data-testid="profile-private-key-row"] [data-testid="nsec-value"]',
  );
  await expect(nsecDisplay).toBeVisible();

  // The mock bridge returns "nsec1mock…"; the display starts masked (blurred).
  await expect(nsecDisplay).toHaveCSS("filter", /blur/);

  // Hide — the nsec display should disappear (state cleared).
  await revealToggle.click();
  await expect(revealToggle).toHaveText("Reveal");
  await expect(nsecDisplay).not.toBeVisible();
});

test("reveal shows error when get_nsec fails", async ({ page }) => {
  await installMockBridge(page, { nsecError: "Keychain locked" });
  await page.goto("/");
  await openSettings(page, "profile");
  await expandIdentity(page);

  const revealToggle = page.getByTestId("profile-private-key-toggle");
  await revealToggle.click();

  // Error text should appear inside the private-key row.
  const keyRow = page.getByTestId("profile-private-key-row");
  await expect(keyRow.locator(".text-destructive")).toBeVisible();
  await expect(keyRow.locator(".text-destructive")).toContainText(
    "Keychain locked",
  );
});

test("settings creates and tests a password-protected key backup", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "profile");
  await expandIdentity(page);

  const row = page.getByTestId("profile-encrypted-backup-row");
  await expect(row).toContainText("Password-protected key backup");
  await page.getByTestId("profile-encrypted-backup-toggle").click();

  await page
    .getByTestId("backup-passphrase-input")
    .fill("mock horse battery staple");
  await page.getByTestId("encrypted-backup-create").click();
  await expect(page.getByTestId("backup-test-dropzone")).toBeVisible();

  await page.getByTestId("backup-test-file-input").setInputFiles({
    name: "identity.ncryptsec",
    mimeType: "text/plain",
    buffer: Buffer.from(
      "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p",
    ),
  });
  await page
    .getByTestId("backup-test-password")
    .fill("mock horse battery staple");
  await expect(page.getByTestId("backup-test-success")).toContainText(
    "Your backup works!",
  );
});
