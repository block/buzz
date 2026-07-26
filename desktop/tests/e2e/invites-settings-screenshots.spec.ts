import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const OUTDIR = "test-results/invites-settings";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    relayRequiresMembership: true,
    relayRole: "owner",
  });
  await page.route("**/api/invites", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      json: {
        code: "community-email-test",
        expires_at: Math.floor(Date.now() / 1000) + 3 * 86_400,
        url: "https://alpha.example.com/invite/community-email-test",
      },
      status: 200,
    });
  });
  await page.goto("/");
  await openSettings(page, "community-members");
});

test("capture: consolidated invites settings", async ({ page }) => {
  const panel = page.getByTestId("settings-panel-community-members");

  await expect(
    page.getByTestId("settings-nav-community-members"),
  ).toContainText("Invites");
  await expect(
    page.getByRole("heading", { name: "Invites", exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("community-icon-settings")).toHaveCount(0);
  await expect(
    page.getByTestId("community-invite-dialog-trigger"),
  ).toBeVisible();
  await expect(page.getByTestId("community-invite-email-field")).toHaveCount(0);
  await expect(page.getByTestId("copy-invite-link")).toHaveCount(0);
  await expect(page.getByText("alice", { exact: true })).toBeVisible();
  await expect(page.getByText("bob", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Manage roles or remove access.", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("People who use the link join as members."),
  ).toHaveCount(0);
  await expect(page.getByTestId("community-icon-save")).toHaveCount(0);

  const aliceName = page.getByText("alice", { exact: true });
  const aliceRow = page
    .locator('[data-testid^="relay-member-row-"]')
    .filter({ has: aliceName });
  const aliceNpub = aliceRow.locator('[data-testid^="relay-member-npub-"]');
  await expect(aliceName).toHaveCSS("opacity", "1");
  await expect(aliceNpub).toHaveCSS("opacity", "0");
  await aliceRow.hover();
  await expect(aliceName).toHaveCSS("opacity", "0");
  await expect(aliceNpub).toHaveCSS("opacity", "1");
  await page.mouse.move(0, 0);

  await waitForAnimations(page);
  await panel.screenshot({ path: `${OUTDIR}/01-invites-settings.png` });
});

test("capture: share-style community invite dialog", async ({ page }) => {
  await page.getByTestId("community-invite-dialog-trigger").click();

  const dialog = page.getByTestId("community-invite-dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Invite to community" }),
  ).toBeVisible();
  await expect(page.getByTestId("community-invite-email-field")).toHaveCount(0);
  await expect(page.getByPlaceholder("Type an email address")).toHaveCount(0);
  await expect(
    dialog.getByRole("heading", { name: "Share with a link" }),
  ).toBeVisible();
  await expect(page.getByTestId("copy-invite-link")).toHaveText("Copy link");
  await expect(page.getByTestId("invite-link-qr-code")).toHaveCount(0);
  await expect(page.getByTestId("invite-link-url")).toHaveCount(0);

  const expiryTrigger = page.getByTestId("invite-link-ttl-trigger");
  await expect(expiryTrigger).toHaveText("3 days");
  await expiryTrigger.click();
  await expect(
    page.getByRole("menuitemradio", { name: "1 day" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitemradio", { name: "30 days" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/02-invite-dialog.png` });
});
