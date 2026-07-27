import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// NIP-IA archive button + "Archived" flair gate matrix.
//
// Guards the composition `canArchive = isSelf || isRelayAdminOrOwner ||
// isOaOwnerOfViewee` in UserProfilePanel.tsx. Unit tests cover each input in
// isolation; this spec covers the OR composition where silent regressions
// (refactor turns OR into AND, role expansion bypasses a branch, etc.) would
// otherwise slip past code review.

const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

async function openSelfProfile(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  // First seed message in #general is from the active identity.
  const firstMessage = page.getByTestId("message-row").first();
  await firstMessage.locator("button", { hasText: "npub1mock..." }).click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
}

async function openAliceProfile(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  // Second seed message in #general is from Alice. Her display name "alice"
  // is registered in mockDisplayNames, so the author button text is "alice".
  const aliceMessage = page.getByTestId("message-row").nth(1);
  await aliceMessage.locator("button", { hasText: "alice" }).first().click();
  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(ALICE_PUBKEY.slice(0, 8));
}

async function openProfileSettingsMenu(page: import("@playwright/test").Page) {
  const trigger = page.getByTestId("user-profile-settings-menu-trigger");
  await expect(trigger).toBeVisible();
  await trigger.click();
}

test.describe("NIP-IA archive button gate", () => {
  test("case 1 — self viewer + self target: Archive visible, no flair", async ({
    page,
  }) => {
    await installMockBridge(page, { relayRole: null, oaOwnerIsMe: false });
    await openSelfProfile(page);
    await openProfileSettingsMenu(page);
    const archiveButton = page.getByTestId("user-profile-archive-identity");
    await expect(archiveButton).toBeVisible();
    await expect(page.getByTestId("user-profile-archived-flair")).toHaveCount(
      0,
    );

    // Archive is now gated behind a confirmation modal — clicking the button
    // opens the dialog rather than firing immediately. Drive the full flow so
    // the gate stays meaningful: the modal must surface, then confirm fires.
    await expect(page.getByTestId("archive-confirm-dialog")).toHaveCount(0);
    await archiveButton.click();
    await expect(page.getByTestId("archive-confirm-dialog")).toBeVisible();
    const confirm = page.getByTestId("archive-confirm-action");
    await expect(confirm).toBeVisible();
    await confirm.click();
    await expect(page.getByTestId("archive-confirm-dialog")).toHaveCount(0);
  });

  test("case 2 — relay admin viewing Alice: Archive visible", async ({
    page,
  }) => {
    await installMockBridge(page, {
      relayRole: "admin",
      relayRequiresMembership: true,
      oaOwnerIsMe: false,
      archivedIdentities: [],
    });
    await openAliceProfile(page);
    await openProfileSettingsMenu(page);
    await expect(
      page.getByTestId("user-profile-archive-identity"),
    ).toBeVisible();
  });

  test("case 3 — verified OA owner viewing Alice: Archive visible", async ({
    page,
  }) => {
    await installMockBridge(page, {
      relayRole: null,
      oaOwnerIsMe: true,
      archivedIdentities: [],
    });
    await openAliceProfile(page);
    await openProfileSettingsMenu(page);
    await expect(
      page.getByTestId("user-profile-archive-identity"),
    ).toBeVisible();
  });

  test("case 4 — no authority viewing Alice: Archive hidden", async ({
    page,
  }) => {
    await installMockBridge(page, {
      relayRole: null,
      oaOwnerIsMe: false,
      archivedIdentities: [],
    });
    await openAliceProfile(page);
    await expect(
      page.getByTestId("user-profile-settings-menu-trigger"),
    ).toHaveCount(0);
    await expect(page.getByTestId("user-profile-archive-identity")).toHaveCount(
      0,
    );
    await expect(
      page.getByTestId("user-profile-unarchive-identity"),
    ).toHaveCount(0);
  });

  test("case 5 — Alice archived: flair + Unarchive button (under admin gate)", async ({
    page,
  }) => {
    await installMockBridge(page, {
      relayRole: "admin",
      relayRequiresMembership: true,
      oaOwnerIsMe: false,
      archivedIdentities: [ALICE_PUBKEY],
    });
    await openAliceProfile(page);
    await expect(page.getByTestId("user-profile-archived-flair")).toBeVisible();
    await openProfileSettingsMenu(page);
    await expect(
      page.getByTestId("user-profile-unarchive-identity"),
    ).toBeVisible();
    await expect(page.getByTestId("user-profile-archive-identity")).toHaveCount(
      0,
    );
  });
});
