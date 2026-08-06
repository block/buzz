import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// The mock identity's own pre-seeded message in #general (authored by
// DEFAULT_MOCK_IDENTITY.pubkey in e2eBridge.ts). Editing/deleting one's own
// message is exactly Sam's workflow: "delete a message by clearing its edit."
const OWN_MESSAGE_ID = "mock-general-welcome";
const ORIGINAL_CONTENT = "Welcome to #general";

// Open the more-actions menu for a message row and wait for the menu to mount.
async function openMoreActionsMenu(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  const row = page.locator(`[data-message-id="${messageId}"]`);
  await row.hover();
  await page.getByTestId(`more-actions-${messageId}`).click();
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });
}

// Enter edit mode for a message, clear it to empty, and submit — the gesture
// that triggers the empty-edit delete confirmation.
async function submitEmptyEdit(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  await openMoreActionsMenu(page, messageId);
  await page.getByTestId(`edit-message-${messageId}`).click();
  await expect(page.getByTestId("edit-target")).toBeVisible({ timeout: 5_000 });
  // Edit mode sets the editor content via Tiptap's async transaction pipeline;
  // wait for it to populate before we clear it.
  const input = page.getByTestId("message-input");
  await expect(input).not.toBeEmpty({ timeout: 5_000 });
  await input.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("Backspace");
  await expect(input).toBeEmpty();
  await page.keyboard.press("Enter");
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("clearing an edit to empty prompts to delete, then deletes on confirm", async ({
  page,
}) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  await submitEmptyEdit(page, OWN_MESSAGE_ID);

  // The same "Delete message?" confirmation the Delete menu action shows — an
  // empty edit is routed through it, not silently deleted.
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await expect(dialog).toContainText("Delete message?");
  // Edit mode stays active while the dialog is open — it exits only on confirm.
  await expect(page.getByTestId("edit-target")).toBeVisible();

  // Confirm → the message row is removed and edit mode has exited.
  await dialog.getByRole("button", { name: "Delete" }).click();
  await expect(dialog).toBeHidden({ timeout: 5_000 });
  await expect(page.getByTestId("edit-target")).toBeHidden();
  await expect(row).toBeHidden({ timeout: 5_000 });
});

test("cancelling the empty-edit delete keeps the message", async ({ page }) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  await submitEmptyEdit(page, OWN_MESSAGE_ID);

  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });

  // Cancel → nothing is deleted, the original message survives, and the user is
  // left in edit mode (the editing session is preserved, not discarded).
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden({ timeout: 5_000 });
  await expect(page.getByTestId("edit-target")).toBeVisible();
  await expect(row).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toContainText(
    ORIGINAL_CONTENT,
  );
});

test("a non-empty edit still edits and never deletes", async ({ page }) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  await openMoreActionsMenu(page, OWN_MESSAGE_ID);
  await page.getByTestId(`edit-message-${OWN_MESSAGE_ID}`).click();
  await expect(page.getByTestId("edit-target")).toBeVisible({ timeout: 5_000 });
  const input = page.getByTestId("message-input");
  await expect(input).not.toBeEmpty({ timeout: 5_000 });
  const editedContent = `Edited, not deleted ${Date.now()}`;

  await input.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(editedContent);
  await page.keyboard.press("Enter");

  // No delete confirmation, edit mode exits, the row survives with new text.
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(page.getByTestId("edit-target")).toBeHidden({ timeout: 10_000 });
  await expect(row).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toContainText(
    editedContent,
  );
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    ORIGINAL_CONTENT,
  );
});
