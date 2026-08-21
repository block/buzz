import { expect, test, bootstrapE2ePage } from "../helpers/test";

import { installMockBridge } from "../helpers/bridge";

// `general` seeds the mock identity as owner, so the owner/admin-gated
// visibility + ephemeral controls are editable. Visibility is a deferred
// draft: selecting a value only updates local state; it persists on Save.
async function openManagementSheet(page: import("@playwright/test").Page) {
  await bootstrapE2ePage(page, "/");
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
  test("01 — lifecycle section: visibility + channel type", async ({
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
    ).toHaveAccessibleName("Visibility: Public");
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

  test("02 — visibility defers to Save", async ({ page }) => {
    await installMockBridge(page, { updateChannelDelayMs: 500 });
    await openManagementSheet(page);
    await openEditDialog(page);

    const lifecycle = page.getByTestId("channel-management-lifecycle");
    await lifecycle.scrollIntoViewIfNeeded();
    const permissions = page.getByTestId("channel-management-permissions");

    // Save starts disabled with no pending edits.
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();

    // Selecting a visibility only updates the local draft: the dialog title
    // reflects the pending choice and Save becomes enabled, but nothing is
    // persisted yet (no "Updating…" state).
    await permissions.click();
    await page
      .getByTestId("channel-management-permissions-option-private")
      .click();
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();
    await expect(permissions).toHaveAccessibleName("Visibility: Private");
    await expect(permissions).not.toHaveAttribute("aria-busy", "true");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeEnabled();
    // Wait for the dropdown to fully close before reopening it, so the
    // trigger stays mounted/stable for the next interaction.
    await expect(
      page.getByTestId("channel-management-permissions-option-private"),
    ).toHaveCount(0);

    // Toggling back to the original value clears the draft and disables Save.
    await permissions.click();
    await page
      .getByTestId("channel-management-permissions-option-open")
      .click();
    await expect(
      page.getByRole("dialog", { name: "Edit public channel" }),
    ).toBeVisible();
    await expect(permissions).toHaveAccessibleName("Visibility: Public");
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();
    await expect(
      page.getByTestId("channel-management-permissions-option-open"),
    ).toHaveCount(0);

    // Choose Private again and commit via Save.
    await permissions.click();
    await page
      .getByTestId("channel-management-permissions-option-private")
      .click();
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

    // Reopen to confirm the visibility change persisted.
    await openEditDialog(page);
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();
    await expect(
      page.getByTestId("channel-management-permissions"),
    ).toHaveAccessibleName("Visibility: Private");
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

  test("05 — owner actions live in the settings list", async ({ page }) => {
    await installMockBridge(page);
    await openManagementSheet(page);

    await expect(page.getByTestId("channel-management-footer")).toHaveCount(0);
    await expect(page.getByTestId("channel-management-archive")).toBeVisible();
    await expect(page.getByTestId("channel-management-delete")).toBeVisible();
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
    ).toHaveAccessibleName("Visibility: Private");
    await expect(
      page.getByTestId("channel-management-channel-type"),
    ).toContainText("Temporary");
    await expect(page.getByTestId("channel-management-ttl")).toContainText(
      "7 days",
    );
    await settle(page);
  });

  test("09 — cancel discards unsaved channel drafts", async ({ page }) => {
    await installMockBridge(page);
    await openManagementSheet(page);
    await openEditDialog(page);

    await page.getByTestId("channel-management-name").fill("discarded-name");
    await page
      .getByRole("textbox", { name: "Description" })
      .fill("This description should be discarded");
    await page.getByTestId("channel-management-permissions").click();
    await page
      .getByTestId("channel-management-permissions-option-private")
      .click();
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();
    await selectTemporaryChannelType(page);
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeEnabled();

    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(
      page.getByRole("dialog", {
        name: /Edit (?:public|private) channel/,
      }),
    ).toHaveCount(0);

    await openEditDialog(page);
    await expect(page.getByTestId("channel-management-name")).toHaveValue(
      "general",
    );
    await expect(
      page.getByRole("textbox", { name: "Description" }),
    ).toHaveValue("General discussion for everyone");
    await expect(
      page.getByRole("dialog", { name: "Edit public channel" }),
    ).toBeVisible();
    await expect(
      page.getByTestId("channel-management-permissions"),
    ).toHaveAccessibleName("Visibility: Public");
    await expect(
      page.getByTestId("channel-management-channel-type"),
    ).toContainText("Ongoing");
    await expect(
      page.getByTestId("channel-management-ephemeral-settings"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("channel-management-save-changes"),
    ).toBeDisabled();
  });

  test("10 — unsaved visibility draft does not leak to a new channel", async ({
    page,
  }) => {
    await installMockBridge(page, { updateChannelDelayMs: 1_500 });
    await openManagementSheet(page);
    await openEditDialog(page);

    const permissions = page.getByTestId("channel-management-permissions");
    await permissions.click();
    await page
      .getByTestId("channel-management-permissions-option-private")
      .click();
    // Draft only — never saved. The dialog title reflects the pending choice.
    await expect(
      page.getByRole("dialog", { name: "Edit private channel" }),
    ).toBeVisible();

    const agentsChannelId = await page
      .getByTestId("channel-agents")
      .getAttribute("data-channel-id");
    if (!agentsChannelId) {
      throw new Error("Expected the agents channel id.");
    }
    await page.evaluate((channelId) => {
      const hash = window.location.hash.replace(/^#/, "") || "/";
      const [, query = ""] = hash.split("?");
      const nextHash = `#/channels/${channelId}${query ? `?${query}` : ""}`;
      window.history.pushState(
        {},
        "",
        `${window.location.pathname}${nextHash}`,
      );
      window.dispatchEvent(new HashChangeEvent("hashchange"));
      window.dispatchEvent(new PopStateEvent("popstate"));
    }, agentsChannelId);
    await expect(page.getByTestId("chat-title")).toHaveText("agents");

    // The unsaved draft must not carry over: the new channel re-syncs from
    // server state and shows its own (public) visibility.
    await expect(
      page.getByRole("dialog", { name: "Edit public channel" }),
    ).toBeVisible();
    await expect(permissions).toHaveAccessibleName("Visibility: Public");
  });
});
