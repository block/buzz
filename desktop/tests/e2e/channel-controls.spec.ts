import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// `general` seeds the mock identity as owner, so the owner/admin-gated
// visibility + ephemeral controls are live and interactive.
async function openManagementSheet(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
}

async function openEditDialog(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-management-edit").click();
  await expect(
    page.getByRole("dialog", {
      name: /Edit (?:public|private) channel/,
    }),
  ).toBeVisible();
}

async function settle(page: import("@playwright/test").Page) {
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((a) => a.finished)),
  );
}

async function selectTemporaryChannelType(
  page: import("@playwright/test").Page,
) {
  await page.getByTestId("channel-management-channel-type").click();
  await page.getByLabel("Temporary channel").click();
}

test.describe("channel controls", () => {
  test("01 — lifecycle section: permissions + channel type", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openManagementSheet(page);
    await openEditDialog(page);
    await expect(
      page.getByRole("dialog", { name: "Edit public channel" }),
    ).toBeVisible();
    await expect(page.getByText(/Update settings for/)).toHaveCount(0);

    const lifecycle = page.getByTestId("channel-management-lifecycle");
    await lifecycle.scrollIntoViewIfNeeded();
    await expect(
      page.getByTestId("channel-management-permissions-container"),
    ).toBeVisible();
    await expect(
      page.getByTestId("channel-management-permissions"),
    ).toHaveAccessibleName("Permissions: Public");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();
    await expect(
      page.getByTestId("channel-management-channel-type"),
    ).toBeVisible();
    await expect(
      page.getByTestId("channel-management-channel-type"),
    ).toContainText("Ongoing");
    await expect(
      page.getByTestId("channel-management-ephemeral-settings"),
    ).toHaveCount(0);
    await expect(
      page
        .getByRole("dialog", {
          name: /Edit (?:public|private) channel/,
        })
        .getByTestId("channel-management-topic"),
    ).toHaveCount(0);
    await expect(
      page
        .getByRole("dialog", {
          name: /Edit (?:public|private) channel/,
        })
        .getByTestId("channel-management-purpose"),
    ).toHaveCount(0);
    await settle(page);
  });

  test("02 — permissions update immediately", async ({ page }) => {
    await installMockBridge(page, { updateChannelDelayMs: 500 });
    await openManagementSheet(page);
    await openEditDialog(page);

    const lifecycle = page.getByTestId("channel-management-lifecycle");
    await lifecycle.scrollIntoViewIfNeeded();
    const permissions = page.getByTestId("channel-management-permissions");
    await permissions.click();
    await page
      .getByTestId("channel-management-permissions-option-private")
      .click();
    await expect(permissions).toHaveAttribute("aria-busy", "true");
    await expect(permissions).toContainText("Updating…");
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();
    await expect(permissions).toHaveAccessibleName("Permissions: Private");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();

    await permissions.click();
    await page
      .getByTestId("channel-management-permissions-option-open")
      .click();
    await expect(
      page.getByRole("dialog", { name: "Edit public channel" }),
    ).toBeVisible();
    await expect(permissions).toHaveAccessibleName("Permissions: Public");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();
    await settle(page);
  });

  test("03 — Temporary type reveals expiration presets", async ({ page }) => {
    await installMockBridge(page);
    await openManagementSheet(page);
    await openEditDialog(page);

    const lifecycle = page.getByTestId("channel-management-lifecycle");
    await lifecycle.scrollIntoViewIfNeeded();
    await selectTemporaryChannelType(page);

    const ephemeralSettings = page.getByTestId(
      "channel-management-ephemeral-settings",
    );
    await expect(ephemeralSettings).toBeVisible();
    const ttl = ephemeralSettings.getByTestId("channel-management-ttl");
    await expect(ttl).toBeVisible();
    await expect(ttl).toHaveAttribute("aria-label", "Expires after");
    await expect(ttl).toContainText("7 days");
    await ttl.click();
    await expect(
      page.getByTestId("channel-management-ttl-option-1800"),
    ).toHaveText("30 minutes");
    await expect(
      page.getByTestId("channel-management-ttl-option-2592000"),
    ).toHaveText("30 days");
    await page.getByTestId("channel-management-ttl-option-86400").click();
    await expect(ttl).toContainText("1 day");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeEnabled();
    await page.getByTestId("channel-management-channel-type").click();
    await page.getByLabel("Ongoing channel").click();
    await expect(ephemeralSettings).toHaveCount(0);
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();
  });

  test("04 — changing the timeout preset keeps save enabled", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openManagementSheet(page);
    await openEditDialog(page);

    const lifecycle = page.getByTestId("channel-management-lifecycle");
    await lifecycle.scrollIntoViewIfNeeded();
    await selectTemporaryChannelType(page);

    const ttl = page.getByTestId("channel-management-ttl");
    await ttl.click();
    await page.getByTestId("channel-management-ttl-option-21600").click();
    await expect(ttl).toContainText("6 hours");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeEnabled();
  });

  test("05 — sticky footer pins lifecycle buttons", async ({ page }) => {
    await installMockBridge(page);
    await openManagementSheet(page);

    const footer = page.getByTestId("channel-management-footer");
    await expect(footer).toBeVisible();
    await expect(page.getByTestId("channel-management-archive")).toBeVisible();
    await settle(page);
  });

  test("06 — full sheet with new controls", async ({ page }) => {
    await installMockBridge(page);
    await openManagementSheet(page);

    const sheet = page.getByTestId("channel-management-sheet");
    await expect(sheet).toBeVisible();
    await settle(page);
  });

  test("07 — saving lifecycle uses unified save", async ({ page }) => {
    await installMockBridge(page, { updateChannelDelayMs: 500 });
    await openManagementSheet(page);
    await openEditDialog(page);

    await selectTemporaryChannelType(page);
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeEnabled();

    await page.getByTestId("channel-management-save-changes").click();
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toHaveText("Saving...");

    await expect(
      page.getByRole("dialog", {
        name: /Edit (?:public|private) channel/,
      }),
    ).toHaveCount(0);
  });

  test("08 — saved ephemeral lifecycle is reflected after reopen", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openManagementSheet(page);
    await openEditDialog(page);

    await page.getByTestId("channel-management-permissions").click();
    await page
      .getByTestId("channel-management-permissions-option-private")
      .click();
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();
    await selectTemporaryChannelType(page);
    await page.getByTestId("channel-management-save-changes").click();
    await expect(
      page.getByRole("dialog", {
        name: /Edit (?:public|private) channel/,
      }),
    ).toHaveCount(0);

    await page.getByTestId("auxiliary-panel-close").click();
    await expect(
      page.getByTestId("channel-management-sheet"),
    ).not.toBeVisible();
    await page.getByTestId("channel-management-trigger").click();
    await openEditDialog(page);
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();

    const lifecycle = page.getByTestId("channel-management-lifecycle");
    await lifecycle.scrollIntoViewIfNeeded();
    await expect(
      page.getByTestId("channel-management-permissions"),
    ).toHaveAccessibleName("Permissions: Private");
    await expect(
      page.getByTestId("channel-management-channel-type"),
    ).toContainText("Temporary");
    await expect(page.getByTestId("channel-management-ttl")).toContainText(
      "7 days",
    );
    await settle(page);
  });
});
