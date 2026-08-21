import { expect, test, bootstrapE2ePage } from "../helpers/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import {
  dropFileOnTestId,
  endWindowFileDrag,
  startWindowFileDrag,
} from "../helpers/fileDrag";

async function enterMachineBackup(page: import("@playwright/test").Page) {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await bootstrapE2ePage(page, "/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
}

async function openBackupOptions(page: import("@playwright/test").Page) {
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await page.getByTestId("backup-options-link").click();
  await expect(
    page.getByTestId("onboarding-page-backup-options"),
  ).toBeVisible();
}

async function openPasswordBackup(page: import("@playwright/test").Page) {
  await openBackupOptions(page);
  await page.getByTestId("backup-option-password").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
}

async function invokedCommands(page: import("@playwright/test").Page) {
  return page.evaluate(
    () =>
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? [],
  );
}

const SHOTS = "test-results/screenshots-onboarding";

// Mirrors the mock bridge's MOCK_NCRYPTSEC (e2eBridge.ts): the blob the
// mocked `create_ncryptsec_backup` returns, i.e. the "downloaded file"
// contents the test-your-backup dropzone expects.
const MOCK_NCRYPTSEC =
  "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

test("backup step appears on fresh-key path after profile submit", async ({
  page,
}) => {
  await enterMachineBackup(page);

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();

  // Perceived-loading intro: the animated logo and "Creating" title show
  // first, then the finished state replaces them after the hold.
  await expect(
    page.getByRole("heading", { name: "Creating your identity key" }),
  ).toBeVisible();
  await expect(page.getByTestId("backup-intro-logo")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();
  await expect(page.getByTestId("onboarding-back")).toBeEnabled();

  await expect(
    page.getByRole("heading", {
      name: "Your unique identity key has been created",
    }),
  ).toBeVisible();
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
});

// ---------------------------------------------------------------------------
// Key-created view: masked key with reveal toggle. Backup options open the
// dark security view; the raw key is fetched only on explicit reveal/copy.
// ---------------------------------------------------------------------------

test("key view reveals explicitly; options copy explicitly", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterMachineBackup(page);

  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);

  // Masked by default: decorative mask only, no key material in the DOM.
  const key = page.getByTestId("backup-key-value");
  await expect(key).toBeVisible();
  await expect(key).toHaveClass(/blur/);
  await expect(key).not.toContainText("nsec1");
  expect(await invokedCommands(page)).not.toContain("get_nsec");

  // Reveal fetches the key; box must not reflow (same-length monospace mask).
  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(key).toContainText("nsec1mock");
  await expect(key).toHaveClass(/select-text/);

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/02-backup-chooser-revealed.png` });

  // Hide again.
  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(key).not.toContainText("nsec1");

  // Copy is available only after opening the dark backup-options view.
  await page.getByTestId("backup-options-link").click();
  await expect(
    page.getByTestId("onboarding-page-backup-options"),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await page.getByTestId("backup-copy-key").click();
  await expect
    .poll(async () => invokedCommands(page))
    .toContain("copy_text_to_clipboard");
  expect(await invokedCommands(page)).toContain("get_nsec");

  // Return restores the yellow key-created view; its Next skips backup and
  // continues directly to setup.
  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Encrypted download path ("Backup your key" step): password → encrypt
// locally → native save → saved confirmation. The raw key must never be
// fetched on this path.
// ---------------------------------------------------------------------------

test("download happy path: generated password, encrypt, native save, Next", async ({
  page,
}) => {
  await enterMachineBackup(page);

  // Password backup opens from the dark options state without adding an
  // onboarding progress step.
  await openPasswordBackup(page);

  // The password field starts empty; the create button sits in the footer's
  // primary slot and stays disabled until a valid password exists.
  const input = page.getByTestId("backup-passphrase-input");
  await expect(input).toHaveValue("");
  await expect(page.getByTestId("encrypted-backup-create")).toBeDisabled();
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await expect(page.getByTestId("backup-return-to-onboarding")).toBeVisible();
  const passwordPanel = page.getByTestId("backup-password-panel");
  await expect(passwordPanel).toBeVisible();
  await expect(passwordPanel).not.toHaveClass(/buzz-card-textured/);
  await expect(passwordPanel).toHaveCSS("padding-left", "24px");

  // The inset refresh icon opens the generator popover and immediately
  // fills the field (mock default: 3 words, spaces).
  await page.getByTestId("backup-passphrase-generate").click();
  await expect(input).toHaveValue("mock horse battery");
  const generatorPopover = page.getByRole("dialog");
  await expect(generatorPopover).toBeVisible();
  await expect(generatorPopover).not.toHaveClass(/buzz-card-textured/);

  // Popover controls regenerate in place: word count (slider) and separator.
  await page.getByTestId("backup-passphrase-words").focus();
  await page.keyboard.press("ArrowRight");
  await expect(input).toHaveValue("mock horse battery staple");
  await page
    .getByTestId("backup-passphrase-separator")
    .selectOption({ label: "Hyphens" });
  await expect(input).toHaveValue("mock-horse-battery-staple");

  // Clicking the inset icon again re-rolls without closing the popover.
  await page.getByTestId("backup-passphrase-generate").click();
  await expect(page.getByTestId("backup-passphrase-separator")).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-backup-download-passphrase.png` });

  // Encryption may still be running when the user commits the download. The
  // explicit click queues the native save without exposing the password or
  // fetching the raw key.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("backup-passphrase-separator")).toHaveCount(0);

  // Saving commits the encrypted payload only after this explicit action.
  await page.getByTestId("encrypted-backup-create").click();

  // Only a successful save (the mock "picks" a path) advances to the
  // "Optionally, test your backup" flow: a select-file button for the saved file
  // (a composer-style drop overlay takes over the card while a file drag is
  // over the window), then the password to unlock it.
  await expect(
    page.getByRole("heading", { name: "Optionally, test your backup" }),
  ).toBeVisible();
  const dropzone = page.getByTestId("backup-test-dropzone");
  await expect(dropzone).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Re-download backup" }),
  ).toBeVisible();

  // The optional security subview has no onboarding Next action. Returning to
  // the yellow key view is the single exit throughout the ceremony.
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await expect(page.getByTestId("backup-return-to-onboarding")).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04-backup-test-dropzone.png` });

  // A wrong file is rejected with an inline error; the dropzone stays.
  await page.getByTestId("backup-test-file-input").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not a key backup"),
  });
  await expect(page.getByTestId("backup-test-error")).toBeVisible();

  // A file drag over the window swaps in the drop overlay; leaving without
  // dropping restores the select button.
  const dropOverlay = page.getByTestId("backup-test-drop-overlay");
  await startWindowFileDrag(page);
  await expect(dropOverlay).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04b-backup-test-drop-overlay.png` });
  await endWindowFileDrag(page);
  await expect(dropOverlay).toHaveCount(0);

  // Dropping the freshly downloaded file on the overlay advances to the
  // password check.
  await startWindowFileDrag(page);
  await expect(dropOverlay).toBeVisible();
  await dropFileOnTestId(page, "backup-test-drop-overlay", MOCK_NCRYPTSEC);
  const password = page.getByTestId("backup-test-password");
  await expect(password).toBeVisible();
  await expect(dropOverlay).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/05-backup-test-password.png` });

  // Verification is explicit and clears every submitted attempt.
  await password.fill("mock-horse-battery-staplX");
  await page.getByTestId("backup-test-verify").click();
  await expect(page.getByTestId("backup-test-error")).toBeVisible();
  await expect(password).toHaveValue("");

  await password.fill("mock horse battery staple lake orbit");
  await page.getByTestId("backup-test-verify").click();
  await expect(page.getByTestId("backup-test-success")).toBeVisible();

  // The celebration is driven by motion's rAF loop, which
  // `waitForAnimations` (WAAPI-only) cannot observe — hold until the badge
  // and copy have faded in before capturing.
  await page.waitForTimeout(1200);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/06-backup-test-success.png` });

  // The download path must never have fetched the raw key.
  const commands = await invokedCommands(page);
  expect(commands).not.toContain("get_nsec");
  expect(commands).toContain("create_ncryptsec_backup");

  // Completion remains inside the optional security subview. Return to the
  // yellow key view, whose standard Next action continues onboarding.
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("security view returns to the yellow onboarding view", async ({
  page,
}) => {
  await enterMachineBackup(page);
  await openPasswordBackup(page);

  await expect(page.getByTestId("backup-passphrase-input")).toBeVisible();
  await expect(page.getByTestId("onboarding-step-dots")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);

  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("backup-key-value")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeVisible();
});

test("returning to onboarding resets password-backup progress", async ({
  page,
}) => {
  await enterMachineBackup(page);
  await openPasswordBackup(page);

  const input = page.getByTestId("backup-passphrase-input");
  await input.fill("mock-horse-battery-staple");
  await page.getByTestId("encrypted-backup-create").click();
  await expect(
    page.getByRole("heading", { name: "Optionally, test your backup" }),
  ).toBeVisible();

  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();

  // Re-entering the security flow intentionally starts a fresh optional
  // backup session so no password or completed state leaks across navigation.
  await openPasswordBackup(page);
  await expect(
    page.getByRole("heading", { name: "Backup your key with a password" }),
  ).toBeVisible();
  await expect(page.getByTestId("backup-passphrase-input")).toHaveValue("");
  await expect(page.getByTestId("encrypted-backup-create")).toBeDisabled();
});

test("typed password requires 12 characters", async ({ page }) => {
  await enterMachineBackup(page);
  await openPasswordBackup(page);

  const create = page.getByTestId("encrypted-backup-create");
  await expect(create).toBeDisabled(); // empty field

  await page.getByTestId("backup-passphrase-input").fill("short");
  await expect(page.getByTestId("backup-passphrase-issue")).toBeVisible();
  await expect(create).toBeDisabled();

  await page
    .getByTestId("backup-passphrase-input")
    .fill("a much longer passphrase");
  await expect(page.getByTestId("backup-passphrase-issue")).toHaveCount(0);
  await expect(create).toBeEnabled();
});

test("backup step back button returns to machine identity choice", async ({
  page,
}) => {
  await enterMachineBackup(page);

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("onboarding-back").click();

  // Backing out preserves the loaded key — primary CTA continues setup rather
  // than minting another identity (#2318).
  await expect(
    page.getByRole("button", { name: "Continue setup" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Use a different key instead" }),
  ).toBeVisible();
});

// ---------------------------------------------------------------------------
// B4: Error path coverage (reveal/copy)
// ---------------------------------------------------------------------------

test("reveal shows inline error when get_nsec fails and Next still advances", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { nsecError: "Keychain locked" },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await bootstrapE2ePage(page, "/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("backup-key-reveal-toggle").click();

  await expect(page.getByTestId("backup-copy-error")).toBeVisible();
  // Keychain failure does not trap the user: Next still skips backup and
  // advances directly to setup.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("reveal retry succeeds after initial failure", async ({ page }) => {
  // First call fails, second succeeds (sequenced via nsecErrors).
  await installMockBridge(
    page,
    { nsecErrors: ["Keychain locked", null] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await bootstrapE2ePage(page, "/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-copy-error")).toBeVisible();

  // Retry — second call succeeds and clears the error.
  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-key-value")).toContainText("nsec1mock");
  await expect(page.getByTestId("backup-copy-error")).not.toBeVisible();
});
