import { expect, test } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { seedActiveIdentity } from "../helpers/onboarding";

const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};

const SHOTS = "test-results/screenshots-onboarding";

test("backup step appears on fresh-key path after profile submit", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Save your private key" }),
  ).toBeVisible();
});

test("backup step shows masked nsec from mock bridge", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  const nsecDisplay = page.getByTestId("nsec-value");
  await expect(nsecDisplay).toBeVisible();

  // Should start masked (blurred) — reveal button exists and eye icon visible.
  const revealBtn = page.getByTestId("nsec-reveal-toggle");
  await expect(revealBtn).toBeVisible();
  await expect(nsecDisplay).toHaveCSS("filter", /blur/);

  // Take a screenshot of the masked state.
  await waitForAnimations(page);
  const backupSection = page.locator('[data-testid="onboarding-page-backup"]');
  await backupSection.screenshot({
    path: `${SHOTS}/02-backup-step-masked.png`,
  });

  // Reveal and verify the mock nsec appears.
  await revealBtn.click();
  await expect(nsecDisplay).not.toHaveCSS("filter", /blur/);
  await expect(nsecDisplay).toContainText("nsec1mock");

  // Take a screenshot of the revealed state.
  await waitForAnimations(page);
  await backupSection.screenshot({
    path: `${SHOTS}/03-backup-step-revealed.png`,
  });
});

test("backup step Next is disabled until checkbox is checked", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("nsec-value")).toBeVisible();

  // Next is disabled while checkbox is unchecked.
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();

  // Check the checkbox → Next enables.
  await page.getByTestId("backup-acknowledge").check();
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
});

test("backup step advances to avatar on Next click", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("nsec-value")).toBeVisible();
  await page.getByTestId("backup-acknowledge").check();
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
});

test("backup step back button returns to profile", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("onboarding-back").click();

  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
});

// ---------------------------------------------------------------------------
// B4: Error path coverage
// ---------------------------------------------------------------------------

test("backup step shows error banner and retry button when get_nsec fails", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    { nsecError: "Keychain locked" },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expect(page.getByTestId("backup-load-error")).toBeVisible();
  await expect(page.getByTestId("backup-retry")).toBeVisible();
  // Next is blocked on error; Skip for now ghost is shown instead.
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();
  await expect(page.getByTestId("backup-skip")).toBeVisible();

  // Skip for now still advances to avatar.
  await page.getByTestId("backup-skip").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
});

test("backup step retry succeeds and shows key after initial failure", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  // First call fails, second succeeds (sequenced via nsecErrors).
  await installMockBridge(
    page,
    { nsecErrors: ["Keychain locked", null] },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("backup-load-error")).toBeVisible();

  // Retry — second call succeeds.
  await page.getByTestId("backup-retry").click();
  await expect(page.getByTestId("nsec-value")).toBeVisible();
  await expect(page.getByTestId("backup-load-error")).not.toBeVisible();
});
