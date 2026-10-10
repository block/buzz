import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function openGeneral(page: Page) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test("keeps the space after a macOS smart quote following a blank line", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { value: "MacIntel" });
  });
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("first");
  await input.press("Shift+Enter");
  await input.press("Shift+Enter");
  await input.pressSequentially("It's ");

  // WebKit's macOS smart-quote substitution arrives after the space as an
  // insertReplacementText event with null data. After two consecutive <br>s,
  // its DOM selection settles immediately before the trailing space instead
  // of after it. Reproduce that native event/selection sequence explicitly so
  // the regression remains testable in CI's Chromium browser.
  await input.evaluate(async (element) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    let lastText: Text | null = null;
    while (walker.nextNode()) lastText = walker.currentNode as Text;
    if (!lastText) throw new Error("missing composer text node");

    element.dispatchEvent(
      new InputEvent("beforeinput", {
        bubbles: true,
        composed: true,
        data: null,
        inputType: "insertReplacementText",
      }),
    );

    lastText.data = lastText.data.replace("'", "’");
    const quoteOffset = lastText.data.indexOf("’") + 1;
    const selection = window.getSelection();
    const replacementRange = document.createRange();
    replacementRange.setStart(lastText, quoteOffset);
    replacementRange.collapse(true);
    selection?.removeAllRanges();
    selection?.addRange(replacementRange);

    element.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        composed: true,
        data: null,
        inputType: "insertReplacementText",
      }),
    );

    const settledRange = document.createRange();
    settledRange.setStart(lastText, lastText.data.length - 1);
    settledRange.collapse(true);
    selection?.removeAllRanges();
    selection?.addRange(settledRange);

    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
  });

  await input.pressSequentially("better");

  await expect(input).toContainText("It’s better");
  await expect(input.locator("br")).toHaveCount(2);
});
