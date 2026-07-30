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

// Enter edit mode for a message and wait until the editor is populated.
async function enterEditMode(
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
  return input;
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("clearing an edit to empty deletes the message", async ({ page }) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  const input = await enterEditMode(page, OWN_MESSAGE_ID);

  // Clear the whole message, then submit the (now empty) edit.
  await input.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("Backspace");
  await expect(input).toBeEmpty();
  await page.keyboard.press("Enter");

  // Empty-edit delete is immediate — no confirmation dialog (unlike the
  // explicit "Delete message" button). Edit mode exits and the row is gone.
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(page.getByTestId("edit-target")).toBeHidden({ timeout: 5_000 });
  await expect(row).toBeHidden({ timeout: 5_000 });
});

test("a non-empty edit still edits and never deletes", async ({ page }) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  const input = await enterEditMode(page, OWN_MESSAGE_ID);
  const editedContent = `Edited, not deleted ${Date.now()}`;

  await input.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(editedContent);
  await page.keyboard.press("Enter");

  // Edit mode exits, the row survives, and its content is the new text — the
  // empty-delete branch must not fire when there is still text.
  await expect(page.getByTestId("edit-target")).toBeHidden({ timeout: 5_000 });
  await expect(row).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toContainText(
    editedContent,
  );
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    ORIGINAL_CONTENT,
  );
});
