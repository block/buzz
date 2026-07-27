import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// ⌘K / Ctrl+K behaviour around the composer:
// - With composer text selected → open the link-edit dialog (the shortcut the
//   formatting toolbar has always advertised on its link button).
// - With a caret inside an existing composer link → open the same dialog
//   seeded with that link.
// - With an empty caret in the composer (no selection, no link) → fall
//   through to the app-wide quick-search dialog.
// - With focus outside the composer → quick search, unchanged.

async function openGeneral(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test("⌘K with selected composer text opens the add-link dialog", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("check out this link");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("ControlOrMeta+k");

  const dialog = page.getByRole("dialog", { name: "Add link" });
  await expect(dialog).toBeVisible();
  // Seeded with the selected text as the display value.
  await expect(dialog.getByLabel("Display text")).toHaveValue(
    "check out this link",
  );
  // Quick search must NOT have opened.
  await expect(page.getByTestId("search-dialog-input")).toHaveCount(0);
});

test("⌘K with caret inside an existing link opens the edit-link dialog", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  // Create a real link through the ⌘K flow first.
  await input.pressSequentially("docs");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("ControlOrMeta+k");
  const addDialog = page.getByRole("dialog", { name: "Add link" });
  await expect(addDialog).toBeVisible();
  await addDialog.getByLabel("URL").fill("https://example.com");
  await addDialog.getByRole("button", { name: "Save" }).click();
  await expect(addDialog).toHaveCount(0);
  await expect(input.locator('a[href="https://example.com"]')).toHaveText(
    "docs",
  );

  // Click into the linked text to place the caret inside it, then re-trigger
  // the shortcut. (The click also surfaces the composer link hover card —
  // ⌘K must open the full dialog from that state.)
  await input.locator('a[href="https://example.com"]').click();
  await page.keyboard.press("ControlOrMeta+k");

  const editDialog = page.getByRole("dialog", { name: "Edit link" });
  await expect(editDialog).toBeVisible();
  await expect(editDialog.getByLabel("URL")).toHaveValue("https://example.com");
  await expect(page.getByTestId("search-dialog-input")).toHaveCount(0);
});

test("⌘K with an empty composer caret still opens quick search", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.press("ControlOrMeta+k");

  await expect(page.getByTestId("search-dialog-input")).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Add link" })).toHaveCount(0);
});

test("⌘K with unselected composer text (caret only) opens quick search", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("draft in progress");
  await page.keyboard.press("ControlOrMeta+k");

  await expect(page.getByTestId("search-dialog-input")).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Add link" })).toHaveCount(0);
});

test("macOS plain Ctrl+K still kill-lines in the composer", async ({
  page,
}) => {
  // Regression guard for the Emacs-style Ctrl-K binding: on macOS the
  // primary-modifier check must reject Control so `macEmacsTextShortcuts`
  // keeps kill-line, and neither the link dialog nor quick search may open.
  test.skip(process.platform !== "darwin", "mac-only Emacs binding");

  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("kill this line");
  // Emacs Ctrl-A → start of line (ProseMirror handles this natively on mac;
  // "Home" is not reliable in headless Chromium).
  await page.keyboard.press("Control+a");
  await page.keyboard.press("Control+k");

  await expect(input).not.toContainText("kill this line");
  await expect(page.getByRole("dialog", { name: "Add link" })).toHaveCount(0);
  await expect(page.getByTestId("search-dialog-input")).toHaveCount(0);
});

test("macOS Ctrl+A, Ctrl+E, and Ctrl+K stay within hard-break lines", async ({
  page,
}) => {
  test.skip(process.platform !== "darwin", "mac-only Emacs bindings");

  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { value: "MacIntel" });
  });
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("first");
  await input.press("Shift+Enter");
  await input.pressSequentially("middle");
  await input.press("Shift+Enter");
  await input.pressSequentially("last");
  await expect(input.locator("br")).toHaveCount(2);

  // Start/end movement applies to the third visual line, not the whole editor.
  await page.keyboard.press("Control+a");
  await input.pressSequentially("[");
  await page.keyboard.press("Control+e");
  await input.pressSequentially("]");
  await expect(input).toHaveText("firstmiddle[last]");

  // Kill only to this line's end, preserving both preceding lines.
  await page.keyboard.press("Control+a");
  await page.keyboard.press("Control+k");
  await expect(input).toHaveText("firstmiddle");
  await expect(input.locator("br")).toHaveCount(3);

  // At end-of-line, kill the newline and join with the following line.
  await page.keyboard.press("Control+b");
  await page.keyboard.press("Control+k");
  await expect(input).toHaveText("firstmiddle");
  await expect(input.locator("br")).toHaveCount(1);
});

test("⌘K outside the composer opens quick search", async ({ page }) => {
  await installMockBridge(page);
  await openGeneral(page);

  // Focus is on the page body — not the composer.
  await page.getByTestId("chat-title").click();
  await page.keyboard.press("ControlOrMeta+k");

  await expect(page.getByTestId("search-dialog-input")).toBeVisible();
});
