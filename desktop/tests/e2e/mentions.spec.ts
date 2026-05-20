import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

/** Locator scoped to the mention autocomplete dropdown inside the composer. */
function autocomplete(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-composer")
    .locator(".rounded-xl.border.bg-popover");
}

test("@ trigger shows autocomplete dropdown with channel members", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await expect(dropdown.getByText("alice")).toBeVisible();
  await expect(dropdown.getByText("bob")).toBeVisible();
});

test("autocomplete filters suggestions as user types", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@ali");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("alice")).toBeVisible();
  await expect(dropdown.getByText("bob")).not.toBeVisible();
});

test("selecting a mention inserts @Name into input", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @ali");

  const dropdown = autocomplete(page);
  await dropdown.getByText("alice").click();

  await expect(input).toHaveText("Hey @alice ");
});

test("mention button opens autocomplete and inserts a selected member", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey ");
  await page.getByTestId("message-insert-mention").click();

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await dropdown.getByText("alice").click();

  await expect(input).toHaveText("Hey @alice ");
});

test("inserting a mention preserves Shift+Enter newlines (regression: bug #2)", async ({
  page,
}) => {
  // Before PR #618, mention insertion round-tripped through
  // `setContent(markdown)`, which collapsed every Shift+Enter hard
  // break to a single space. After the fix, autocomplete uses a
  // native ProseMirror `tr.insertText` transaction and the line
  // breaks survive.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("line one");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("line two @ali");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("alice")).toBeVisible();
  await dropdown.getByText("alice").click();

  // Both lines must still be present, separated by a real line break
  // (rendered as a `<br>` by Tiptap; the projection sees `\n`).
  await expect(input).toHaveText(/line one[\s\S]*line two @alice/);
  await expect(input.locator("br")).toHaveCount(1);
});

test("keyboard navigation selects mention with Enter", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@ali");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("alice")).toBeVisible();

  // Press Enter to select the first (and only) suggestion
  await input.press("Enter");

  // Should insert @alice and NOT send the message
  await expect(input).toHaveText("@alice ");
});

test("Escape dismisses autocomplete dropdown", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();

  await input.press("Escape");

  await expect(dropdown).not.toBeVisible();
});

test("mention text is highlighted in sent messages", async ({ page }) => {
  const message = `Hey @alice check this out ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill(message);
  await page.getByTestId("send-message").click();

  // The mention should render inside a styled span with the mention class
  const mentionSpan = page
    .getByTestId("message-row")
    .last()
    .locator("span.text-primary", { hasText: "@alice" });
  await expect(mentionSpan).toBeVisible();
});

test("clicking author name opens user profile panel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // The seed message in general is from the mock identity (npub1mock...)
  const firstMessage = page.getByTestId("message-row").first();
  const authorButton = firstMessage.locator("button", {
    hasText: "npub1mock...",
  });
  await authorButton.click();

  // Click now opens the full profile panel instead of the popover
  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("deadbeef");
});

test("hovering avatar opens popover, clicking opens profile panel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const firstMessage = page.getByTestId("message-row").first();
  const avatarButton = firstMessage.locator("button").first();

  // Hover should open the popover
  await avatarButton.hover();
  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();

  // Click should close the popover and open the profile panel
  await avatarButton.click();
  await expect(profilePopover).toHaveCount(0);
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
});
