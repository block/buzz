import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function openGeneral(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function selectText(input: Locator, selectedText: string) {
  await input.evaluate((element, text) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    let offset = 0;

    while (walker.nextNode()) {
      const node = walker.currentNode;
      const value = node.textContent ?? "";
      const index = value.indexOf(text);
      if (index >= 0) {
        const range = document.createRange();
        range.setStart(node, index);
        range.setEnd(node, index + text.length);

        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        (element as HTMLElement).focus();
        document.dispatchEvent(new Event("selectionchange"));
        return;
      }
      offset += value.length;
    }

    throw new Error(
      `Could not select "${text}" in composer after ${offset} characters`,
    );
  }, selectedText);
}

async function dragSelectText(
  page: Page,
  input: Locator,
  selectedText: string,
  backward = false,
) {
  const points = await input.evaluate((element, text) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);

    while (walker.nextNode()) {
      const node = walker.currentNode;
      const value = node.textContent ?? "";
      const index = value.indexOf(text);
      if (index < 0) continue;

      const startRange = document.createRange();
      startRange.setStart(node, index);
      startRange.setEnd(node, index + 1);
      const startRect = startRange.getBoundingClientRect();

      const endRange = document.createRange();
      endRange.setStart(node, index + text.length - 1);
      endRange.setEnd(node, index + text.length);
      const endRect = endRange.getBoundingClientRect();

      return {
        start: {
          x: startRect.left + 1,
          y: startRect.top + startRect.height / 2,
        },
        end: {
          x: endRect.right - 1,
          y: endRect.top + endRect.height / 2,
        },
      };
    }

    throw new Error(`Could not locate "${text}" for mouse selection`);
  }, selectedText);

  const dragStart = backward ? points.end : points.start;
  const dragEnd = backward ? points.start : points.end;
  await page.mouse.move(dragStart.x, dragStart.y);
  await page.mouse.down();
  await page.mouse.move(dragEnd.x, dragEnd.y, { steps: 12 });
  await page.mouse.up();

  await expect
    .poll(() => page.evaluate(() => window.getSelection()?.toString()))
    .toBe(selectedText);
}

async function applySelectionFormat(
  page: Page,
  input: Locator,
  label: "Bullet list" | "Code block" | "Ordered list",
  collapseAfterMouseDown = false,
  useMouseSelection = false,
) {
  if (useMouseSelection) {
    await dragSelectText(page, input, "selected");
  } else {
    await selectText(input, "selected");
  }
  const tray = page.getByTestId("selection-formatting-tray");
  await expect(tray).toBeVisible();
  const button = tray.getByRole("button", { name: label });

  if (collapseAfterMouseDown) {
    await button.evaluate((element, inputTestId) => {
      element.addEventListener(
        "mouseup",
        () => {
          const input = document.querySelector(
            `[data-testid="${inputTestId}"]`,
          );
          if (!input) throw new Error("Composer input not found");

          const range = document.createRange();
          range.selectNodeContents(input);
          range.collapse(false);
          const selection = window.getSelection();
          selection?.removeAllRanges();
          selection?.addRange(range);
          document.dispatchEvent(new Event("selectionchange"));
        },
        { once: true },
      );
    }, "message-input");
  }

  await button.click();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

for (const format of [
  { label: "Code block", selector: "pre" },
  { label: "Bullet list", selector: "ul" },
  { label: "Ordered list", selector: "ol" },
] as const) {
  test(`${format.label} applies only to the selected composer text`, async ({
    page,
  }) => {
    await openGeneral(page);

    const input = page.getByTestId("message-input");
    await input.fill("before selected after");
    await applySelectionFormat(page, input, format.label);

    await expect(input.locator(":scope > p").first()).toHaveText("before ");
    await expect(input.locator(`:scope > ${format.selector}`)).toHaveText(
      "selected",
    );
    await expect(input.locator(":scope > p").last()).toHaveText(" after");
    await expect(input).toHaveText("before selected after");
  });
}

test("block formatting preserves the lines around a selected composer line", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("before");
  await input.press("Shift+Enter");
  await input.pressSequentially("selected");
  await input.press("Shift+Enter");
  await input.pressSequentially("after");
  await applySelectionFormat(page, input, "Bullet list");

  await expect(input.locator(":scope > p").first()).toHaveText("before");
  await expect(input.locator(":scope > ul")).toHaveText("selected");
  await expect(input.locator(":scope > p").last()).toHaveText("after");

  await page.getByTestId("send-message").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_SIGNED_EVENTS__?: Array<{ content: string }>;
            }
          ).__BUZZ_E2E_SIGNED_EVENTS__?.at(-1)?.content,
      ),
    )
    .toBe("before\n\n- selected\n\nafter");
});

test("block formatting restores a selection collapsed by the toolbar interaction", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.fill("before selected after");
  await applySelectionFormat(page, input, "Bullet list", true);

  await expect(input.locator(":scope > p").first()).toHaveText("before ");
  await expect(input.locator(":scope > ul")).toHaveText("selected");
  await expect(input.locator(":scope > p").last()).toHaveText(" after");
});

test("block formatting only changes text selected with a native mouse drag", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.fill("before selected after");
  await applySelectionFormat(page, input, "Bullet list", false, true);

  await expect(input.locator(":scope > p").first()).toHaveText("before ");
  await expect(input.locator(":scope > ul")).toHaveText("selected");
  await expect(input.locator(":scope > p").last()).toHaveText(" after");
});

test("block formatting preserves a backward native selection", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.fill("before selected after");
  await dragSelectText(page, input, "selected", true);

  const tray = page.getByTestId("selection-formatting-tray");
  await expect(tray).toBeVisible();
  await tray.getByRole("button", { name: "Bullet list" }).click();

  await expect(input.locator(":scope > ul")).toHaveText("selected");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const selection = window.getSelection();
        if (!(selection?.anchorNode && selection.focusNode)) return false;
        const anchorRange = document.createRange();
        anchorRange.setStart(selection.anchorNode, selection.anchorOffset);
        anchorRange.collapse(true);
        const focusRange = document.createRange();
        focusRange.setStart(selection.focusNode, selection.focusOffset);
        focusRange.collapse(true);
        return (
          anchorRange.compareBoundaryPoints(Range.START_TO_START, focusRange) >
          0
        );
      }),
    )
    .toBe(true);
});

test("Buzz theme uses the primary color for the selection formatter", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.fill("before selected after");
  await selectText(input, "selected");
  const tray = page.getByTestId("selection-formatting-tray");
  await expect(tray).toBeVisible();

  const colors = await tray.evaluate((element) => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = "hsl(var(--primary))";
    probe.style.color = "hsl(var(--primary-foreground))";
    document.body.appendChild(probe);
    const probeStyles = getComputedStyle(probe);
    const trayStyles = getComputedStyle(element);
    const result = {
      primaryBackground: probeStyles.backgroundColor,
      primaryForeground: probeStyles.color,
      trayBackground: trayStyles.backgroundColor,
      trayForeground: trayStyles.color,
    };
    probe.remove();
    return result;
  });

  expect(colors.trayBackground).toBe(colors.primaryBackground);
  expect(colors.trayForeground).toBe(colors.primaryForeground);
});
