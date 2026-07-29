import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

async function enterMachineBackup(page: import("@playwright/test").Page) {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
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

  await expect(
    page.getByRole("heading", {
      name: "Your unique identity key has been created",
    }),
  ).toBeVisible();
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Chooser: masked key with reveal toggle and inline copy. The raw key is
// fetched only on explicit reveal/copy, and Next is never blocked.
// ---------------------------------------------------------------------------

test("chooser shows masked key; reveal and copy fetch it explicitly", async ({
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

  // Inline copy goes straight to the clipboard.
  await page.getByTestId("backup-copy-key").click();
  await expect
    .poll(async () => invokedCommands(page))
    .toContain("copy_text_to_clipboard");
  expect(await invokedCommands(page)).toContain("get_nsec");

  // Next leads into the download step, where backup stays skippable.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
  await page.getByTestId("onboarding-skip").click();
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

  // The download flow is its own onboarding step behind the footer's Next.
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();

  // The password field starts empty; the create button sits in the footer's
  // primary slot and stays disabled until a valid password exists.
  const input = page.getByTestId("backup-passphrase-input");
  await expect(input).toHaveValue("");
  await expect(page.getByTestId("encrypted-backup-create")).toBeDisabled();
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-skip")).toBeVisible();

  // The inset refresh icon opens the generator popover and immediately
  // fills the field (mock default: 3 words, spaces).
  await page.getByTestId("backup-passphrase-generate").click();
  await expect(input).toHaveValue("mock horse battery");

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

  // Esc closes the popover; the generated password stays in the field.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("backup-passphrase-separator")).toHaveCount(0);
  await expect(input).toHaveValue("mock-horse-battery-staple");

  await page.getByTestId("encrypted-backup-create").click();

  // Only a successful save (the mock "picks" a path) advances to the
  // "Now, test your backup" flow: a select-file button (dropzone while
  // dragging) for the saved file, then the password to unlock it.
  await expect(
    page.getByRole("heading", { name: "Now, test your backup" }),
  ).toBeVisible();
  const dropzone = page.getByTestId("backup-test-dropzone");
  await expect(dropzone).toBeVisible();
  await expect(page.getByTestId("encrypted-backup-saved-path")).toContainText(
    "identity.ncryptsec",
  );

  // Until the test passes there is no Next at all — Skip is the only way
  // forward.
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-skip")).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04-backup-test-dropzone.png` });

  // A wrong file is rejected with an inline error; the dropzone stays.
  await page.getByTestId("backup-test-file-input").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not a key backup"),
  });
  await expect(page.getByTestId("backup-test-file-error")).toBeVisible();

  // The freshly downloaded file advances to the password check.
  await page.getByTestId("backup-test-file-input").setInputFiles({
    name: "identity.ncryptsec",
    mimeType: "text/plain",
    buffer: Buffer.from(MOCK_NCRYPTSEC),
  });
  const password = page.getByTestId("backup-test-password");
  await expect(password).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/05-backup-test-password.png` });

  // A full-length wrong password shows the mismatch hint, never success.
  await password.fill("mock-horse-battery-staplX");
  await expect(page.getByTestId("backup-test-password-mismatch")).toBeVisible();
  await expect(page.getByTestId("backup-test-success")).toHaveCount(0);

  // Typing the password completely succeeds without any extra click.
  await password.fill("mock-horse-battery-staple");
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

  // A passed test unlocks Next and retires the Skip escape hatch.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await expect(page.getByTestId("onboarding-skip")).toHaveCount(0);
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("download step Back returns to the backup chooser", async ({ page }) => {
  await enterMachineBackup(page);

  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
  await expect(page.getByTestId("backup-passphrase-input")).toBeVisible();
  // The chooser's footer Next belongs to the previous step; the download
  // step only mounts its own Next once the backup exists.
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);

  await page.getByTestId("onboarding-back").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("backup-key-value")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeVisible();
});

test("test-view Back returns to the password form with the password intact", async ({
  page,
}) => {
  await enterMachineBackup(page);
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();

  const input = page.getByTestId("backup-passphrase-input");
  await input.fill("mock-horse-battery-staple");
  await page.getByTestId("encrypted-backup-create").click();
  await expect(
    page.getByRole("heading", { name: "Now, test your backup" }),
  ).toBeVisible();

  // Back from the test view rolls back to the password form (same step),
  // keeping the entered password; only from the form does Back leave the
  // step.
  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Backup your key with a password" }),
  ).toBeVisible();
  await expect(input).toHaveValue("mock-horse-battery-staple");

  // Re-downloading runs the ceremony again instantly from the cached
  // encryption.
  await page.getByTestId("encrypted-backup-create").click();
  await expect(
    page.getByRole("heading", { name: "Now, test your backup" }),
  ).toBeVisible();

  await page.getByTestId("onboarding-back").click();
  await page.getByTestId("onboarding-back").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
});

test("typed password requires 12 characters", async ({ page }) => {
  await enterMachineBackup(page);
  await page.getByTestId("onboarding-next").click();

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
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("backup-key-reveal-toggle").click();

  await expect(page.getByTestId("backup-copy-error")).toBeVisible();
  // Keychain failure does not trap the user: Next still advances into the
  // download step, and Skip there continues to setup.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
  await page.getByTestId("onboarding-skip").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("reveal retry succeeds after initial failure", async ({ page }) => {
  // First call fails, second succeeds (sequenced via nsecErrors).
  await installMockBridge(
    page,
    { nsecErrors: ["Keychain locked", null] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-copy-error")).toBeVisible();

  // Retry — second call succeeds and clears the error.
  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-key-value")).toContainText("nsec1mock");
  await expect(page.getByTestId("backup-copy-error")).not.toBeVisible();
});
