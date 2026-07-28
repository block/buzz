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
// Keycase path: password → create locally → native save → saved confirmation.
// The raw key must never be fetched on this path.
// ---------------------------------------------------------------------------

test("Keycase happy path: generated password, create, native save, Next", async ({
  page,
}) => {
  await enterMachineBackup(page);

  // Default mode: generated password shown, but backup remains optional.
  await expect(page.getByTestId("backup-passphrase-generated")).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();

  // Let the perceived-loading intro (animated logo → content fade-in) finish
  // so the screenshot captures fully opaque content.
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/02-backup-step-passphrase.png` });

  await page.getByTestId("encrypted-backup-create").click();

  // The locally created blob stays masked; the portable save action is explicit.
  const blob = page.getByTestId("ncryptsec-value");
  await expect(blob).toBeVisible();
  await expect(blob).toHaveCSS("filter", /blur/);
  await page.getByTestId("ncryptsec-reveal-toggle").click();
  await expect(blob).toContainText("ncryptsec1");

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-backup-step-encrypted.png` });

  await expect(page.getByTestId("encrypted-backup-saved-path")).toContainText(
    "identity.ncryptsec",
  );

  // The default path must never have fetched the raw key.
  const commands = await page.evaluate(
    () =>
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).not.toContain("get_nsec");
  expect(commands).toContain("create_ncryptsec_backup");

  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("custom passphrase requires 12 characters and confirmation", async ({
  page,
}) => {
  await enterMachineBackup(page);

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

// ---------------------------------------------------------------------------
// Raw-key path: preserved behind one explicit advanced action.
// ---------------------------------------------------------------------------

test("raw key path is one explicit click away and shows the masked nsec", async ({
  page,
}) => {
  await enterMachineBackup(page);

  await page.getByTestId("backup-show-raw-key").click();

  const nsecDisplay = page.getByTestId("nsec-value");
  await expect(nsecDisplay).toBeVisible();

  // Should start masked (blurred) — reveal button exists and eye icon visible.
  const revealBtn = page.getByTestId("nsec-reveal-toggle");
  await expect(revealBtn).toBeVisible();
  await expect(nsecDisplay).toHaveCSS("filter", /blur/);

  // Reveal and verify the mock nsec appears.
  await revealBtn.click();
  await expect(nsecDisplay).not.toHaveCSS("filter", /blur/);
  await expect(nsecDisplay).toContainText("nsec1mock");

  // Intro crossfade must be finished before capturing.
  await expect(page.getByTestId("backup-intro-logo")).toHaveCount(0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04-backup-step-raw-revealed.png` });

  // Next remains enabled on the advanced raw-key path.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
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
// B4: Error path coverage (raw path)
// ---------------------------------------------------------------------------

test("raw path shows error banner and retry button when get_nsec fails", async ({
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
  await page.getByTestId("backup-show-raw-key").click();

  await expect(page.getByTestId("backup-load-error")).toBeVisible();
  await expect(page.getByTestId("backup-retry")).toBeVisible();
  // Keychain failure does not trap the user; both Next and explicit skip work.
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await expect(page.getByTestId("backup-skip")).toBeVisible();

  // Skip for now still advances to machine setup.
  await page.getByTestId("backup-skip").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("raw path retry succeeds and shows key after initial failure", async ({
  page,
}) => {
  // First call fails, second succeeds (sequenced via nsecErrors).
  await installMockBridge(
    page,
    { nsecErrors: ["Keychain locked", null] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByTestId("backup-show-raw-key").click();

  await expect(page.getByTestId("backup-load-error")).toBeVisible();

  // Retry — second call succeeds.
  await page.getByTestId("backup-retry").click();
  await expect(page.getByTestId("nsec-value")).toBeVisible();
  await expect(page.getByTestId("backup-load-error")).not.toBeVisible();
});
