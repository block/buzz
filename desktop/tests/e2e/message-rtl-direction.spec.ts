import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Regression cover for RTL message direction. The app root is `ltr`, so before
// the fix every block inherited it: a Hebrew message rendered as an LTR
// paragraph — left-aligned, and long enough to wrap it broke into the wrong
// visual reading order. Blocks whose chrome flanks the text (list markers, the
// quote bar) additionally need `direction` itself flipped, not just the bidi
// paragraph, or the markers strand on the far side of right-aligned text.
//
// These assert on layout facts rather than CSS declarations: line-box edges for
// alignment, and which physical side the inline padding resolved to. A rule
// that stops applying — or gets overridden downstream — fails here even if the
// stylesheet still contains it.

const CHANNEL = "general";

const HEBREW_PARAGRAPH =
  "שאלה טובה — ויש חדשות טובות וחצי. זה משפט ארוך מספיק כדי להישבר לשתי שורות לפחות בתוך הבועה הזאת.";
const ENGLISH_PARAGRAPH =
  "An English paragraph inside the same message keeps its own direction.";
const HEBREW_MESSAGE = [
  HEBREW_PARAGRAPH,
  "",
  "- פריט ראשון ברשימה בעברית",
  "- פריט שני",
  "",
  "> ציטוט בעברית",
  "",
  ENGLISH_PARAGRAPH,
].join("\n");

type Edges = { left: number; right: number }[];

// A paragraph is right-flush (RTL) when its line boxes share a right edge and
// vary on the left, and left-flush (LTR) when the reverse holds. One-line
// paragraphs cannot show this, so callers pass text long enough to wrap.
function isFlush(edges: Edges, side: "left" | "right") {
  if (edges.length < 2) throw new Error("need a wrapped paragraph to compare");
  const spread = (values: number[]) =>
    Math.max(...values) - Math.min(...values);
  const flush = edges.map((edge) => edge[side]);
  const ragged = edges.map((edge) => edge[side === "left" ? "right" : "left"]);
  return spread(flush) <= 1 && spread(ragged) > 1;
}

async function paragraphEdges(page: Page, text: string): Promise<Edges> {
  return page.evaluate((needle) => {
    const paragraph = [
      ...document.querySelectorAll<HTMLElement>(".message-markdown p"),
    ].find((node) => node.textContent?.includes(needle.slice(0, 24)));
    if (!paragraph)
      throw new Error(`no rendered paragraph matching: ${needle}`);
    const range = document.createRange();
    range.selectNodeContents(paragraph);
    return [...range.getClientRects()].map((rect) => ({
      left: Math.round(rect.left),
      right: Math.round(rect.right),
    }));
  }, text);
}

async function emitHebrewMessage(page: Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(
    ({ channelName, content }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({ channelName, content });
    },
    { channelName: CHANNEL, content: HEBREW_MESSAGE },
  );
  await expect(page.getByTestId("message-timeline")).toContainText("פריט שני");
}

test.describe("RTL message direction", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
    await page.setViewportSize({ width: 900, height: 700 });
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL}`).click();
    await expect(page.getByTestId("message-timeline")).toBeVisible();
  });

  test("a Hebrew paragraph renders right-flush and an English one stays left-flush", async ({
    page,
  }) => {
    await emitHebrewMessage(page);

    expect(isFlush(await paragraphEdges(page, HEBREW_PARAGRAPH), "right")).toBe(
      true,
    );

    // Same message, opposite script: per-block resolution, not one direction
    // stamped on the whole message.
    const english = await paragraphEdges(page, ENGLISH_PARAGRAPH);
    expect(english[0].left).toBeLessThan(english[0].right);
    expect(
      await page.evaluate((needle) => {
        const paragraph = [
          ...document.querySelectorAll<HTMLElement>(".message-markdown p"),
        ].find((node) => node.textContent?.includes(needle.slice(0, 24)));
        return paragraph ? getComputedStyle(paragraph).unicodeBidi : null;
      }, ENGLISH_PARAGRAPH),
    ).toBe("plaintext");
  });

  test("Hebrew list markers and the quote bar move to the right edge", async ({
    page,
  }) => {
    await emitHebrewMessage(page);

    const chrome = await page.evaluate((marker) => {
      // Scope to our own message: `general` ships with seeded English content,
      // so an unscoped `.message-markdown ul` can match somebody else's list.
      const body = [
        ...document.querySelectorAll<HTMLElement>(".message-markdown"),
      ].find((node) => node.textContent?.includes(marker));
      if (!body) throw new Error("no rendered body for the Hebrew message");
      const list = body.querySelector<HTMLElement>("ul");
      const quote = body.querySelector<HTMLElement>("blockquote");
      if (!list || !quote) throw new Error("missing list or blockquote");
      const listStyle = getComputedStyle(list);
      const quoteStyle = getComputedStyle(quote);
      return {
        listDirection: listStyle.direction,
        listPaddingLeft: Number.parseFloat(listStyle.paddingLeft),
        listPaddingRight: Number.parseFloat(listStyle.paddingRight),
        quoteDirection: quoteStyle.direction,
        quoteBorderLeft: Number.parseFloat(quoteStyle.borderLeftWidth),
        quoteBorderRight: Number.parseFloat(quoteStyle.borderRightWidth),
      };
    }, "פריט שני");

    expect(chrome.listDirection).toBe("rtl");
    expect(chrome.listPaddingRight).toBeGreaterThan(chrome.listPaddingLeft);
    expect(chrome.quoteDirection).toBe("rtl");
    expect(chrome.quoteBorderRight).toBeGreaterThan(chrome.quoteBorderLeft);
  });

  test("the composer follows the draft's own script", async ({ page }) => {
    const editor = page.getByTestId("message-input");
    const direction = () =>
      editor.evaluate((node) => getComputedStyle(node).direction);

    await editor.click();
    await page.keyboard.type(HEBREW_PARAGRAPH);
    await expect.poll(direction).toBe("rtl");

    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.press("Backspace");
    await page.keyboard.type(ENGLISH_PARAGRAPH);
    await expect.poll(direction).toBe("ltr");
  });

  test("a Hebrew list typed in the composer indents from the right", async ({
    page,
  }) => {
    const editor = page.getByTestId("message-input");
    await editor.click();
    await page.keyboard.type("- פריט ראשון ברשימה");

    await expect
      .poll(() =>
        editor.evaluate((node) => {
          const list = node.querySelector<HTMLElement>("ul, ol");
          if (!list) return null;
          const style = getComputedStyle(list);
          return (
            Number.parseFloat(style.paddingInlineStart) > 0 &&
            Number.parseFloat(style.paddingRight) >
              Number.parseFloat(style.paddingLeft)
          );
        }),
      )
      .toBe(true);
  });
});
