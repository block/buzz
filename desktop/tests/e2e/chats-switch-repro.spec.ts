import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("switching chats does not stack headers", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/#/chats");

  const composer = page.locator("[contenteditable='true'], textarea").first();
  await expect(composer).toBeVisible();
  await composer.click();
  await composer.fill("First chat about apples");
  await composer.press("Enter");
  await expect(page).toHaveURL(/\/chats\/.+/, { timeout: 10_000 });

  // Back to the new-chat screen, then create a second chat.
  await page
    .getByRole("button", { name: "New chat without a project" })
    .click();
  const composer2 = page.locator("[contenteditable='true'], textarea").first();
  await composer2.click();
  await composer2.fill("Second chat about bananas");
  await composer2.press("Enter");
  await expect(page).toHaveURL(/\/chats\/.+/, { timeout: 10_000 });

  const first = page.getByRole("button", {
    exact: true,
    name: "First chat about apples",
  });
  const second = page.getByRole("button", {
    exact: true,
    name: "Second chat about bananas",
  });

  for (let i = 0; i < 5; i++) {
    await first.click();
    await page.waitForTimeout(150);
    await second.click();
    await page.waitForTimeout(150);
  }

  const headerCount = await page.getByTestId("chat-header").count();
  console.log("chat-header count after switches:", headerCount);
  await page.screenshot({
    path: "test-results/chats-switch-repro.png",
    fullPage: false,
  });
  expect(headerCount).toBe(1);

  // Leaving the Chats tab and returning restores the last-viewed chat
  // instead of defaulting to the new-chat screen.
  const lastChatId = page.url().split("/chats/")[1];
  expect(lastChatId).toBeTruthy();
  await page.getByTestId("open-agents-view").click();
  await expect(page).toHaveURL(/agents/);
  await page.getByTestId("open-chats-view").click();
  await expect(page).toHaveURL(new RegExp(`/chats/${lastChatId}`));
  await expect(page.getByTestId("chat-title")).toContainText(
    "Second chat about bananas",
  );

  // Right-click offers rename/pin/archive; pinning moves the chat to the top
  // of its section.
  await first.click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Rename chat" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Archive chat" }),
  ).toBeVisible();
  await page.getByRole("menuitem", { name: "Pin chat" }).click();
  const chatNames = page
    .getByRole("button", { name: /chat about/ })
    .filter({ hasNotText: "Archive" });
  await expect(chatNames.first()).toHaveText(/First chat about apples/);

  // Rename through the context menu updates the sidebar and header.
  await first.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Rename chat" }).click();
  const renameInput = page.getByLabel("Chat name");
  await renameInput.fill("Apple planning");
  await page.getByRole("button", { name: "Rename", exact: true }).click();
  await expect(
    page.getByRole("button", { exact: true, name: "Apple planning" }),
  ).toBeVisible();
});
