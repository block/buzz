import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import {
  dropFileOnTestId,
  endWindowFileDrag,
  startWindowFileDrag,
} from "../helpers/fileDrag";

async function enterMachineBackup(
  page: import("@playwright/test").Page,
  mock?: Parameters<typeof installMockBridge>[1],
) {
  await installMockBridge(page, mock, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();
}

test("fresh-key path explains the identity key before creating it", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await expect(page.getByTestId("onboarding-content-card")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Create a private identity key" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-key-guidance").locator("p"),
  ).toHaveText([
    "Stored securely on this device",
    "Never share it—anyone with this key can sign in as you",
    "Use a secure backup to recover your account",
  ]);
  await expect(page.getByTestId("onboarding-page-backup")).toHaveCount(0);

  await page.getByRole("button", { name: "Create my private key" }).click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
});

test("identity creation failures stay visible on the intro page", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      identityReadErrorAfter: {
        message: "Keychain is unavailable",
        successfulReads: 1,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();

  await expect(page.getByTestId("identity-key-create-error")).toContainText(
    "Keychain is unavailable",
  );
  await expect(
    page.getByRole("button", { name: "Create my private key" }),
  ).toBeEnabled();
  await expect(page.getByTestId("onboarding-page-key-intro")).toBeVisible();
});

async function openPasswordBackup(page: import("@playwright/test").Page) {
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
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
      name: "Your private identity key",
    }),
  ).toBeVisible();
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
});

// ---------------------------------------------------------------------------
// Key-created view: visible key with a hover-to-copy action.
// ---------------------------------------------------------------------------

test("key view shows the key and reveals Copy on hover", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterMachineBackup(page);

  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);

  const key = page.getByTestId("backup-key-value");
  await expect(key).toBeVisible();
  await expect(key).toContainText("nsec1mock");
  await expect(key).toHaveCSS("filter", "none");
  expect(await invokedCommands(page)).toContain("get_nsec");

  // The backup action gains a subtle surface on hover without shifting its
  // content or changing the resting state.
  const backupOption = page.getByTestId("backup-option-password");
  const backupOptionBox = await backupOption.boundingBox();
  const [backupIconBox, backupChevronBox] = await Promise.all([
    backupOption.locator("svg").first().boundingBox(),
    backupOption.locator("svg").last().boundingBox(),
  ]);
  if (!backupOptionBox || !backupIconBox || !backupChevronBox) {
    throw new Error("Could not measure locked-backup row padding");
  }
  expect(backupIconBox.x - backupOptionBox.x).toBeGreaterThanOrEqual(12);
  expect(
    backupOptionBox.x +
      backupOptionBox.width -
      (backupChevronBox.x + backupChevronBox.width),
  ).toBeGreaterThanOrEqual(12);
  await expect(backupOption).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await backupOption.hover();
  await expect(backupOption).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );

  // Hovering the clean key well obscures its contents and reveals Copy
  // without another keychain read.
  const keyWell = page.getByTestId("backup-key-well");
  const copyButton = page.getByTestId("backup-copy-key");
  const readsBeforeHover = (await invokedCommands(page)).filter(
    (command) => command === "get_nsec",
  ).length;
  await expect(copyButton).toHaveCSS("opacity", "0");
  await keyWell.hover();
  await expect(key).toHaveCSS("filter", /blur\(4px\)/);
  await expect(copyButton).toHaveCSS("opacity", "1");
  expect(
    (await invokedCommands(page)).filter((command) => command === "get_nsec"),
  ).toHaveLength(readsBeforeHover);

  await copyButton.click();
  await expect(copyButton).toContainText("Copied to clipboard");
  await expect
    .poll(async () => invokedCommands(page))
    .toContain("copy_text_to_clipboard");
  await expect(key).toContainText("nsec1mock");

  // The primary action continues directly to setup.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Encrypted download path ("Backup your key" step): password → encrypt
// locally → native save → saved confirmation.
// ---------------------------------------------------------------------------

test("download happy path: generated password, encrypt, native save, Next", async ({
  page,
}) => {
  await enterMachineBackup(page);

  // Password backup stays inside the onboarding card without adding a generic
  // Next action.
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
  await expect(passwordPanel).toHaveCSS("padding-left", "0px");
  await expect(page.getByTestId("backup-password-timeline")).toHaveCount(0);
  await expect(
    passwordPanel.getByText("Password", { exact: true }),
  ).toBeVisible();
  const subtitle = page.getByText(
    "This creates a password-protected file with your private key. Remember, Buzz can’t recover your key if you lose it.",
  );
  const passwordLabel = passwordPanel.getByText("Password", { exact: true });
  const [subtitleBox, passwordLabelBox] = await Promise.all([
    subtitle.boundingBox(),
    passwordLabel.boundingBox(),
  ]);
  expect(subtitleBox).not.toBeNull();
  expect(passwordLabelBox).not.toBeNull();
  expect(
    (passwordLabelBox?.y ?? 0) -
      ((subtitleBox?.y ?? 0) + (subtitleBox?.height ?? 0)),
  ).toBeLessThanOrEqual(96);
  await expect(input).toHaveCSS("height", "48px");
  await expect(input).toHaveCSS("text-align", "left");
  await expect(input).toHaveCSS("background-color", "rgb(249, 249, 249)");

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
  // explicit click queues the native save without exposing the password.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("backup-passphrase-separator")).toHaveCount(0);

  // Saving commits the encrypted payload only after this explicit action.
  await page.getByTestId("encrypted-backup-create").click();

  // Only a successful save (the mock "picks" a path) advances to the
  // "Your backup is ready" flow: a select-file button for the saved file
  // (a composer-style drop overlay takes over the card while a file drag is
  // over the window), then the password to unlock it.
  await expect(
    page.getByRole("heading", { name: "Your backup is ready" }),
  ).toBeVisible();
  const dropzone = page.getByTestId("backup-test-dropzone");
  await expect(dropzone).toBeVisible();
  await expect(dropzone).toHaveText("Test your backup");
  await expect(dropzone).toHaveClass(/w-full/);
  const [dropzoneBox, backupPanelBox] = await Promise.all([
    dropzone.boundingBox(),
    passwordPanel.boundingBox(),
  ]);
  expect(dropzoneBox).not.toBeNull();
  expect(backupPanelBox).not.toBeNull();
  expect(dropzoneBox?.width ?? 0).toBeGreaterThanOrEqual(
    (backupPanelBox?.width ?? 0) * 0.95,
  );
  await expect(
    page.getByRole("button", { name: "Download backup again" }),
  ).toBeVisible();
  await expect(page.getByTestId("encrypted-backup-save-copy")).toHaveClass(
    /w-full/,
  );

  // The optional security subview has no onboarding Next action. Returning to
  // the key-created view is the single exit throughout the ceremony.
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

  // The generated-key view reads the key once for its default display; the
  // encrypted backup path still uses its dedicated native command.
  const commands = await invokedCommands(page);
  expect(commands).toContain("get_nsec");
  expect(commands).toContain("create_ncryptsec_backup");

  // Completion remains inside the optional security subview. Return to the
  // key-created view, whose standard Next action continues onboarding.
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("pending backup action collapses to an in-button loader", async ({
  page,
}) => {
  await enterMachineBackup(page, { backupEncryptionDelayMs: 3_000 });
  await openPasswordBackup(page);

  await page
    .getByTestId("backup-passphrase-input")
    .fill("mock-horse-battery-staple");
  const create = page.getByTestId("encrypted-backup-create");
  const readyButtonBox = await create.boundingBox();
  await create.click();

  await expect(create).toHaveAttribute("aria-busy", "true");
  await expect(create).toHaveAccessibleName("Encrypting your key");
  await expect(create.getByTestId("encrypted-backup-encrypting")).toBeVisible();
  const pendingButtonBox = await create.boundingBox();
  if (!readyButtonBox || !pendingButtonBox) {
    throw new Error("Could not measure the encrypted-backup action button");
  }
  expect(pendingButtonBox.height).toBeCloseTo(readyButtonBox.height, 0);
  expect(pendingButtonBox.width).toBeCloseTo(pendingButtonBox.height, 0);
  expect(pendingButtonBox.width).toBeLessThan(readyButtonBox.width);
  await expect(
    page.getByText("Encrypting your password", { exact: true }),
  ).toHaveCount(0);

  await expect(
    page.getByRole("heading", { name: "Your backup is ready" }),
  ).toBeVisible();
});

test("security view returns to the identity-key onboarding view", async ({
  page,
}) => {
  await enterMachineBackup(page);
  await openPasswordBackup(page);

  await expect(page.getByTestId("backup-passphrase-input")).toBeVisible();
  await expect(page.getByTestId("onboarding-step-dots")).toBeVisible();
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
    page.getByRole("heading", { name: "Your backup is ready" }),
  ).toBeVisible();

  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();

  // Re-entering the security flow intentionally starts a fresh optional
  // backup session so no password or completed state leaks across navigation.
  await openPasswordBackup(page);
  await expect(
    page.getByRole("heading", { name: "Create a secure backup file" }),
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

test("backup step back button returns through key guidance to identity choice", async ({
  page,
}) => {
  await enterMachineBackup(page);

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Create a private identity key" }),
  ).toBeVisible();
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
// B4: Error path coverage (copy)
// ---------------------------------------------------------------------------

test("copy shows inline error when get_nsec fails and Next still advances", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { nsecError: "Keychain locked" },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("backup-key-well").hover();
  await page.getByTestId("backup-copy-key").click();

  await expect(page.getByTestId("backup-copy-error")).toBeVisible();
  // Keychain failure does not trap the user: Next still skips backup and
  // advances directly to setup.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("Copy retries after an initial key read fails", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  // The default key read fails first; explicit copy retries and succeeds.
  await installMockBridge(
    page,
    { nsecErrors: ["Keychain locked", null] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();

  await expect(page.getByTestId("backup-copy-error")).toBeVisible();
  await expect(page.getByTestId("backup-key-value")).not.toContainText(
    "nsec1mock",
  );

  // Copy retries the read, clears the error, and restores the default visible
  // key treatment.
  await page.getByTestId("backup-key-well").hover();
  await page.getByTestId("backup-copy-key").click();
  await expect(page.getByTestId("backup-copy-key")).toContainText(
    "Copied to clipboard",
  );
  await expect(page.getByTestId("backup-key-value")).toContainText("nsec1mock");
  await expect(page.getByTestId("backup-copy-error")).not.toBeVisible();
});
