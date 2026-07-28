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

  // Backup is recommended, never required.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Encrypted download path: password → encrypt locally → native save → saved
// confirmation. The raw key must never be fetched on this path.
// ---------------------------------------------------------------------------

test("download happy path: generated password, encrypt, native save, Next", async ({
  page,
}) => {
  await enterMachineBackup(page);

  // The download flow sits behind the footer CTA.
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await page.getByTestId("backup-option-download").click();

  // Default mode: generated password shown, but backup remains optional.
  await expect(page.getByTestId("backup-passphrase-generated")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-backup-download-passphrase.png` });

  await page.getByTestId("encrypted-backup-create").click();

  // The locally created blob stays masked; the portable save action is explicit.
  const blob = page.getByTestId("ncryptsec-value");
  await expect(blob).toBeVisible();
  await expect(blob).toHaveCSS("filter", /blur/);
  await page.getByTestId("ncryptsec-reveal-toggle").click();
  await expect(blob).toContainText("ncryptsec1");

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04-backup-download-encrypted.png` });

  await expect(page.getByTestId("encrypted-backup-saved-path")).toContainText(
    "identity.ncryptsec",
  );

  // The download path must never have fetched the raw key.
  const commands = await invokedCommands(page);
  expect(commands).not.toContain("get_nsec");
  expect(commands).toContain("create_ncryptsec_backup");

  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("download view returns to the chooser via All backup options", async ({
  page,
}) => {
  await enterMachineBackup(page);

  await page.getByTestId("backup-option-download").click();
  await expect(page.getByTestId("backup-passphrase-generated")).toBeVisible();
  // The footer download CTA hides while the flow is open.
  await expect(page.getByTestId("backup-option-download")).toHaveCount(0);

  await page.getByTestId("backup-back-to-options").click();
  await expect(page.getByTestId("backup-key-value")).toBeVisible();
  await expect(page.getByTestId("backup-option-download")).toBeVisible();
});

test("custom passphrase requires 12 characters and confirmation", async ({
  page,
}) => {
  await enterMachineBackup(page);
  await page.getByTestId("backup-option-download").click();

  await page.getByTestId("backup-passphrase-choose-own").click();
  const create = page.getByTestId("encrypted-backup-create");

  await page.getByTestId("backup-passphrase-custom").fill("short");
  await expect(page.getByTestId("backup-passphrase-issue")).toBeVisible();
  await expect(create).toBeDisabled();

  await page
    .getByTestId("backup-passphrase-custom")
    .fill("a much longer passphrase");
  await expect(create).toBeDisabled(); // confirm still empty

  await page
    .getByTestId("backup-passphrase-confirm")
    .fill("a much longer passphrase");
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
  // Keychain failure does not trap the user.
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
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-copy-error")).toBeVisible();

  // Retry — second call succeeds and clears the error.
  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-key-value")).toContainText("nsec1mock");
  await expect(page.getByTestId("backup-copy-error")).not.toBeVisible();
});
