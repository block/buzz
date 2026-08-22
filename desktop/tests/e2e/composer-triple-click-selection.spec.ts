import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function openGeneral(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function tripleClickText(page: Page, input: Locator, text: string) {
  const point = await input.evaluate((element, targetText) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);

    while (walker.nextNode()) {
      const node = walker.currentNode;
      const value = node.textContent ?? "";
      const index = value.indexOf(targetText);
      if (index < 0) continue;

      const range = document.createRange();
      range.setStart(node, index);
      range.setEnd(node, index + targetText.length);
      const rect = range.getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    }

    throw new Error(`Could not locate "${targetText}" for triple-click`);
  }, text);

  await page.mouse.click(point.x, point.y, { clickCount: 3 });
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("triple-clicking a composer line selects only that line, not the whole message", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("before");
  await input.press("Shift+Enter");
  await input.pressSequentially("selected line");
  await input.press("Shift+Enter");
  await input.pressSequentially("after");

  await tripleClickText(page, input, "selected line");

  await expect
    .poll(() => page.evaluate(() => window.getSelection()?.toString()))
    .toBe("selected line");
});
