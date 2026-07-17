import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test } from "@playwright/test";
import { nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { seedActiveIdentity } from "../helpers/onboarding";

const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};

const SHOTS = "test-results/screenshots-onboarding";

test("avatar step always shows Skip for now button without an error", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();

  // Skip button must be visible before any avatar is chosen (no error path).
  const skipBtn = page.getByTestId("onboarding-skip");
  await expect(skipBtn).toBeVisible();
  await expect(skipBtn).toBeEnabled();
  await expect(skipBtn).toHaveText("Skip for now");

  // Capture the whole viewport: the Skip/Next/Back CTAs are portaled into the
  // docked footer (a sibling of the step subtree), so a section-scoped shot
  // would omit the very buttons this artifact is meant to show.
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/01-avatar-skip-button.png`,
  });
});

test("avatar step skip button completes community profile setup", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page.getByTestId("onboarding-skip").click();

  await expect(page.getByTestId("onboarding-gate")).not.toBeVisible();
});

test("avatar Next button still requires an avatar to be chosen", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();

  // Next is disabled until an avatar is set.
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();

  // Once an avatar URL is provided, Next enables.
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/avatar.png");
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
});

// ---------------------------------------------------------------------------
// B4: Routing tests
// ---------------------------------------------------------------------------

test("import-key path skips backup and goes directly to avatar", async ({
  page,
}) => {
  // Import tyler's OWN key (same pubkey = no component remount) so the
  // identityWasImported flag persists in the same component instance.
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  // Profile page — click "Use existing key" to open the key import form.
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await page.getByTestId("onboarding-import-key").click();
  await expect(
    page.getByRole("heading", { name: "Use your existing key" }),
  ).toBeVisible();

  // Enter tyler's own nsec (same pubkey → no remount, identityWasImported stays true).
  const tylerNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.tyler.privateKey));
  await page.getByTestId("nostr-import-nsec-input").fill(tylerNsec);
  await expect(page.getByTestId("nostr-import-npub-preview")).toBeVisible();
  await page.getByTestId("nostr-import-submit").click();

  // After import, the flow returns to profile with identityWasImported=true.
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  // Backup page must NOT appear — avatar comes next on the import path.
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await expect(page.getByTestId("onboarding-page-backup")).not.toBeVisible();
});

test("Back from the community avatar step returns to profile", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page.getByTestId("onboarding-back").click();

  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
});
