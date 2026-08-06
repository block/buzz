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

async function selectTextRange(
  input: Locator,
  firstText: string,
  lastText: string,
) {
  await input.evaluate(
    (element, texts) => {
      const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
      let first: Text | null = null;
      let last: Text | null = null;
      while (walker.nextNode()) {
        const node = walker.currentNode as Text;
        if (!first && node.data.includes(texts.firstText)) first = node;
        if (node.data.includes(texts.lastText)) last = node;
      }
      if (!(first && last))
        throw new Error("Could not find selection endpoints");
      const range = document.createRange();
      range.setStart(first, first.data.indexOf(texts.firstText));
      range.setEnd(
        last,
        last.data.indexOf(texts.lastText) + texts.lastText.length,
      );
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      (element as HTMLElement).focus();
      document.dispatchEvent(new Event("selectionchange"));
    },
    { firstText, lastText },
  );
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
  label: "Bullet list" | "Code block" | "Ordered list" | "Quote",
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

async function applyCaretFormat(
  page: Page,
  label: "Bullet list" | "Code block" | "Ordered list" | "Quote",
) {
  await page.getByRole("button", { name: "Toggle formatting" }).first().click();
  await page.getByRole("button", { name: label, exact: true }).click();
}

for (const platform of [
  { name: "macOS", navigatorPlatform: "MacIntel", shortcut: "Meta+Shift+V" },
  {
    name: "Windows/Linux",
    navigatorPlatform: "Win32",
    shortcut: "Control+Shift+V",
  },
]) {
  test(`pastes rich clipboard content without formatting on ${platform.name}`, async ({
    page,
  }) => {
    await page.addInitScript((navigatorPlatform) => {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        value: navigatorPlatform,
      });
    }, platform.navigatorPlatform);
    await page
      .context()
      .grantPermissions(["clipboard-read", "clipboard-write"], {
        origin: "http://127.0.0.1:4173",
      });
    await openGeneral(page);

    await page.evaluate(async () => {
      await navigator.clipboard.write([
        new ClipboardItem({
          "text/html": new Blob(
            [
              '<p><strong>Bold</strong> and <a href="https://example.com">linked</a></p><ul><li>list item</li></ul>',
            ],
            { type: "text/html" },
          ),
          "text/plain": new Blob(["Bold and linked\nlist item"], {
            type: "text/plain",
          }),
        }),
      ]);
    });

    const input = page.getByTestId("message-input");
    await input.click();
    await page.keyboard.press(platform.shortcut);

    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
              .__BUZZ_E2E_COMMANDS__ ?? [],
        ),
      )
      .toContain("read_clipboard_text");
    await expect(input).toHaveText("Bold and linkedlist item");
    await expect(input.locator("strong, a, ul, li")).toHaveCount(0);
    await expect(input.locator(":scope > p")).toHaveText([
      "Bold and linked",
      "list item",
    ]);
  });
}

for (const format of [
  { label: "Code block", selector: "pre" },
  { label: "Bullet list", selector: "ul" },
  { label: "Ordered list", selector: "ol" },
  { label: "Quote", selector: "blockquote" },
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

  test(`${format.label} starts at a collapsed caret on a new line`, async ({
    page,
  }) => {
    await openGeneral(page);

    const input = page.getByTestId("message-input");
    await input.click();
    await input.pressSequentially("before");
    await input.press("Shift+Enter");
    await applyCaretFormat(page, format.label);
    await input.pressSequentially("inside");

    await expect(input.locator(":scope > p").first()).toHaveText("before");
    await expect(input.locator(`:scope > ${format.selector}`)).toHaveText(
      "inside",
    );
  });

  test(`${format.label} at a collapsed caret formats only the caret's line`, async ({
    page,
  }) => {
    await openGeneral(page);

    const input = page.getByTestId("message-input");
    await input.click();
    await input.pressSequentially("before");
    await input.press("Shift+Enter");
    await input.pressSequentially("target");
    await input.press("Shift+Enter");
    await input.pressSequentially("after");
    // Collapse the caret into the middle line.
    await selectText(input, "target");
    await input.press("ArrowRight");
    await applyCaretFormat(page, format.label);

    await expect(input.locator(":scope > p").first()).toHaveText("before");
    await expect(input.locator(`:scope > ${format.selector}`)).toHaveText(
      "target",
    );
    await expect(input.locator(":scope > p").last()).toHaveText("after");
  });
}

test("Code block uses the restored multiline selection after mouseup collapse", async ({
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
  await applySelectionFormat(page, input, "Code block", true);

  await expect(input.locator(":scope > p").first()).toHaveText("before");
  await expect(input.locator(":scope > pre")).toHaveText("selected");
  await expect(input.locator(":scope > p").last()).toHaveText("after");
});

for (const list of [
  { label: "Bullet list", selector: "ul" },
  { label: "Ordered list", selector: "ol" },
] as const) {
  test(`selected hard-break lines become separate ${list.label.toLowerCase()} items`, async ({
    page,
  }) => {
    await openGeneral(page);
    const input = page.getByTestId("message-input");
    await input.click();
    await input.pressSequentially("one");
    await input.press("Shift+Enter");
    await input.pressSequentially("two");
    await input.press("Shift+Enter");
    await input.pressSequentially("three");
    await selectTextRange(input, "one", "three");
    await page
      .getByTestId("selection-formatting-tray")
      .getByRole("button", { name: list.label })
      .click();

    const items = input.locator(`:scope > ${list.selector} > li`);
    await expect(items).toHaveCount(3);
    await expect(items).toHaveText(["one", "two", "three"]);
  });
}

test("partial list-item selections snap to whole items for block formats", async ({
  page,
}) => {
  for (const format of [
    "Code block",
    "Bullet list",
    "Ordered list",
    "Quote",
  ] as const) {
    await openGeneral(page);
    const input = page.getByTestId("message-input");
    await input.click();
    await input.pressSequentially("before");
    await input.press("Shift+Enter");
    await input.pressSequentially("first");
    await input.press("Shift+Enter");
    await input.pressSequentially("second");
    await input.press("Shift+Enter");
    await input.pressSequentially("after");
    await selectTextRange(input, "before", "after");
    await page
      .getByTestId("selection-formatting-tray")
      .getByRole("button", { name: "Bullet list" })
      .click();

    await selectTextRange(input, "irst", "seco");
    await page
      .getByTestId("selection-formatting-tray")
      .getByRole("button", { name: format })
      .click();

    const structure = await input.locator(":scope > *").evaluateAll((nodes) =>
      nodes.map((node) => ({
        tag: node.tagName.toLowerCase(),
        text: node.textContent,
        items: Array.from(
          node.querySelectorAll(":scope > li"),
          (item) => item.textContent,
        ),
      })),
    );
    const expected = {
      "Code block": [
        { tag: "ul", text: "before", items: ["before"] },
        { tag: "pre", text: "first\nsecond", items: [] },
        { tag: "ul", text: "after", items: ["after"] },
      ],
      "Bullet list": [
        {
          tag: "ul",
          text: "beforefirstsecondafter",
          items: ["before", "first", "second", "after"],
        },
      ],
      "Ordered list": [
        { tag: "ul", text: "before", items: ["before"] },
        { tag: "ol", text: "firstsecond", items: ["first", "second"] },
        { tag: "ul", text: "after", items: ["after"] },
      ],
      Quote: [
        { tag: "ul", text: "before", items: ["before"] },
        { tag: "blockquote", text: "firstsecond", items: [] },
        { tag: "ul", text: "after", items: ["after"] },
      ],
    }[format];
    expect(structure).toEqual(expected);
    await page.reload();
  }
});

test("selected hard-break lines stay newline-separated in one code block", async ({
  page,
}) => {
  await openGeneral(page);
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("one");
  await input.press("Shift+Enter");
  await input.pressSequentially("two");
  await input.press("Shift+Enter");
  await input.pressSequentially("three");
  await selectTextRange(input, "one", "three");
  await page
    .getByTestId("selection-formatting-tray")
    .getByRole("button", { name: "Code block" })
    .click();

  await expect(input.locator(":scope > pre")).toHaveCount(1);
  await expect(input.locator(":scope > pre")).toHaveText("one\ntwo\nthree");

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
    .toBe("```\none\ntwo\nthree\n```");
});

test("selected list items become one multiline code block and keep neighbors", async ({
  page,
}) => {
  await openGeneral(page);
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("before");
  await input.press("Shift+Enter");
  await input.pressSequentially("one");
  await input.press("Shift+Enter");
  await input.pressSequentially("two");
  await input.press("Shift+Enter");
  await input.pressSequentially("after");
  await selectTextRange(input, "before", "after");
  await page
    .getByTestId("selection-formatting-tray")
    .getByRole("button", { name: "Bullet list" })
    .click();
  await selectTextRange(input, "one", "two");
  await page
    .getByTestId("selection-formatting-tray")
    .getByRole("button", { name: "Code block" })
    .click();

  await expect(input.locator(":scope > pre")).toHaveCount(1);
  await expect(input.locator(":scope > pre")).toHaveText("one\ntwo");
  await expect(input.locator(":scope > ul li")).toHaveText(["before", "after"]);

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
    .toBe("- before\n\n```\none\ntwo\n```\n\n- after");
});

test("caret-only block formatting serializes the prior draft unchanged", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("before");
  await input.press("Shift+Enter");
  await applyCaretFormat(page, "Bullet list");
  await input.pressSequentially("item");

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
    .toBe("before\n\n- item");
});

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
