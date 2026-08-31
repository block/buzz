import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

/**
 * Regression guard for #3219: a "ghost" caret rendered one line above the
 * real blinking caret after two consecutive Shift+Enter presses at
 * end-of-line. The bug is a Tauri-WebView compositor paint artifact — a
 * headless Chromium run under Playwright is very unlikely to reproduce the
 * visual glitch itself, so this spec cannot assert the double-caret glyph.
 * Instead it asserts the underlying document structure is correct: two hard
 * breaks are inserted (not extra/duplicated), and typed text after both
 * breaks lands as the true last content — i.e. the caret actually ended up
 * after both breaks, not stuck behind one of them. The visual repaint fix
 * (`requestCaretRepaint` in useRichTextEditor.ts) needs manual verification
 * on a real packaged Tauri build.
 */

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("two consecutive Shift+Enter presses land the caret after both hard breaks", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("first line");
  await input.press("Shift+Enter");
  await input.press("Shift+Enter");
  await input.pressSequentially("third line");

  const result = await input.evaluate((el) => {
    // The composer wraps its content in a single <p> (StarterKit's
    // paragraph node); descend into it so `lastChild` is the trailing text
    // node after the hard breaks, not the outer contentEditable wrapper
    // (whose own lastChild is that same <p>, with a textContent spanning
    // the whole line).
    const paragraph = el.querySelector("p") ?? el;
    return {
      breakCount: el.querySelectorAll("br").length,
      lastTextContent: paragraph.lastChild?.textContent ?? paragraph.textContent,
    };
  });

  // Exactly two hard breaks — no phantom extra break node left behind by
  // the stock hard-break end-of-parent workaround.
  expect(result.breakCount).toBe(2);
  // The caret truly landed after both breaks: "third line" is the last
  // text content, not swallowed into an earlier line.
  expect(result.lastTextContent).toBe("third line");

  await expect(input).toHaveText("first linethird line");
});
