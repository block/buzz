import { expect, test, type Page } from "@playwright/test";

/**
 * Composition-input spike.
 *
 * Answers one question before the composer is designed: does staged character
 * input — Japanese, Korean, Chinese — survive directly beside an inline atom
 * tile? These languages assemble characters in a composition session before
 * committing them, and an inline non-editable box is where that assembly is
 * documented to break (lexical#7985, lexical#6296).
 *
 * `page.keyboard.type()` produces NO composition events, so it proves nothing
 * here. This drives the real CDP IME surface (`Input.imeSetComposition`) plus
 * `Input.insertText` for the commit, which is what a platform IME actually
 * does to the webview.
 */

const SPIKE = "/spike/composer";

type Ime = {
  setComposition: (text: string) => Promise<void>;
  commit: (text: string) => Promise<void>;
};

/** Real composition events through Chrome DevTools Protocol. */
async function imeFor(page: Page): Promise<Ime> {
  const session = await page.context().newCDPSession(page);
  return {
    async setComposition(text: string) {
      await session.send("Input.imeSetComposition", {
        text,
        selectionStart: text.length,
        selectionEnd: text.length,
      });
    },
    async commit(text: string) {
      await session.send("Input.insertText", { text });
    },
  };
}

async function openSpike(page: Page) {
  await page.goto(SPIKE);
  const editor = page.getByTestId("spike-editor");
  await expect(editor).toBeVisible();
  await editor.click();
  return editor;
}

type SpikeRead = { json: unknown; text: string; addresses: string[] };

/** Live editor state, read straight from the editor rather than a React readout. */
function read(page: Page): Promise<SpikeRead> {
  return page.evaluate(() => {
    const reader = window.__SPIKE_READ__;
    if (!reader) throw new Error("Spike harness is not mounted");
    return reader();
  });
}

async function tileCount(page: Page): Promise<number> {
  return (await read(page)).addresses.length;
}

async function addresses(page: Page): Promise<string[]> {
  return (await read(page)).addresses;
}

async function text(page: Page): Promise<string> {
  return (await read(page)).text;
}

test.describe("composition input beside an inline tile", () => {
  test("composes Japanese immediately after a tile", async ({ page }) => {
    await openSpike(page);
    const ime = await imeFor(page);

    await page.getByTestId("insert-morgan").click();
    expect(await tileCount(page)).toBe(1);
    const before = await addresses(page);

    // Compose "にほんご" the way an IME does: staged, then committed.
    await ime.setComposition("に");
    await ime.setComposition("にほ");
    await ime.setComposition("にほん");
    await ime.commit("にほんご");

    expect(await tileCount(page)).toBe(1);
    expect(await addresses(page)).toEqual(before);
    expect(await text(page)).toContain("にほんご");
  });

  test("composes Korean immediately before a leading tile", async ({
    page,
  }) => {
    await openSpike(page);
    const ime = await imeFor(page);

    await page.getByTestId("insert-morgan").click();
    const before = await addresses(page);

    // Caret to the very start, in front of the tile that opens the document.
    await page.keyboard.press("Home");

    await ime.setComposition("ㅎ");
    await ime.setComposition("하");
    await ime.commit("한국어");

    expect(await tileCount(page)).toBe(1);
    expect(await addresses(page)).toEqual(before);
    expect(await text(page)).toContain("한국어");
  });

  test("composes Chinese between two adjacent tiles", async ({ page }) => {
    await openSpike(page);
    const ime = await imeFor(page);

    await page.getByTestId("insert-morgan").click();
    await page.getByTestId("insert-alex").click();
    expect(await tileCount(page)).toBe(2);
    const before = await addresses(page);

    // Land between the two tiles: one press back from the end.
    await page.keyboard.press("ArrowLeft");

    await ime.setComposition("z");
    await ime.setComposition("zh");
    await ime.commit("中文");

    expect(await tileCount(page)).toBe(2);
    expect(await addresses(page)).toEqual(before);
    expect(await text(page)).toContain("中文");
  });

  test("backspace mid-composition beside a tile leaves the tile intact", async ({
    page,
  }) => {
    await openSpike(page);
    const ime = await imeFor(page);

    await page.getByTestId("insert-morgan").click();
    const before = await addresses(page);

    await ime.setComposition("に");
    await ime.setComposition("にほ");
    // Cancel the composition mid-flight the way a user backing out does.
    await ime.setComposition("");
    await page.keyboard.press("Backspace");

    // The tile may or may not be deleted by that Backspace — either is a
    // defensible product choice. What must not happen is a corrupted document.
    const after = await addresses(page);
    expect(after.length === before.length || after.length === 0).toBe(true);
    for (const address of after) expect(before).toContain(address);
  });

  test("committed composition does not alter a tile's address", async ({
    page,
  }) => {
    await openSpike(page);
    const ime = await imeFor(page);

    await page.getByTestId("insert-morgan").click();
    await ime.setComposition("に");
    await ime.commit("にほんご");
    await page.keyboard.type(" hello");

    // The address is the binding. Text around it must never rewrite it.
    expect(await addresses(page)).toEqual(["person/pk-morgan"]);
    expect(await text(page)).toContain("buzz://person/pk-morgan");
  });
});

test.describe("caret boundaries around an inline tile", () => {
  // EXPECTED FAILURE until the caret-boundary layer lands. Chrome places the
  // caret after a leading inline atom regardless of intent — ProseMirror's
  // author confirms the library cannot fix it (discuss #2538). ArrowLeft does
  // reach the position; Home and a pointer click do not. Semi Design injects
  // zero-width characters and Atlassian ships cursor-target spans to repair
  // exactly this. When that layer exists these flip green and the annotation
  // must be removed.
  test("Home then typing lands before a leading tile", async ({ page }) => {
    test.fail();
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    await page.keyboard.press("Home");
    await page.keyboard.type("Hey ");

    const body = await text(page);
    expect(body.indexOf("Hey")).toBeLessThan(
      body.indexOf("buzz://person/pk-morgan"),
    );
  });

  test("clicking a leading tile's left edge lands before it", async ({
    page,
  }) => {
    test.fail();
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    const box = await page.locator(".spike-tile").first().boundingBox();
    if (!box) throw new Error("Tile is not laid out");
    await page.mouse.click(box.x + 2, box.y + box.height / 2);
    await page.keyboard.type("L");

    const body = await text(page);
    expect(body.indexOf("L")).toBeLessThan(
      body.indexOf("buzz://person/pk-morgan"),
    );
  });

  test("ArrowLeft reaches the position before a leading tile", async ({
    page,
  }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    await page.keyboard.press("ArrowLeft");
    await page.keyboard.type("Hey ");

    const body = await text(page);
    expect(body.indexOf("Hey")).toBeLessThan(
      body.indexOf("buzz://person/pk-morgan"),
    );
  });

  // Typing between two adjacent tiles is NONDETERMINISTIC in Chrome without a
  // boundary layer: across repeated runs the text sometimes lands between the
  // tiles and sometimes after both. Asserting a position here would be a flaky
  // test, so this asserts the invariant that always holds — the document is
  // never corrupted — and the boundary layer owns making the position reliable.
  test("typing between two adjacent tiles never corrupts the document", async ({
    page,
  }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();
    await page.getByTestId("insert-alex").click();

    await page.keyboard.press("ArrowLeft");
    await page.keyboard.type("and");

    expect(await tileCount(page)).toBe(2);
    expect(await addresses(page)).toEqual([
      "person/pk-morgan",
      "person/pk-alex",
    ]);
    expect(await text(page)).toContain("and");
  });

  // Binds the single-object property the whole design rests on. Note it does
  // NOT bind the `atom: true` flag: a contentless node is already a leaf and
  // therefore already atomic (ProseMirror computes `isAtom` as
  // `isLeaf || spec.atom`). What this does bind is `selectable: false` —
  // flipping that turns this red, because a selected tile is replaced by the
  // next keystroke.
  test("the caret cannot be placed inside a tile", async ({ page }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    // Aim at the visual middle of the label — inside it, if that were possible.
    const box = await page.locator(".spike-tile").first().boundingBox();
    if (!box) throw new Error("Tile is not laid out");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.type("ZZ");

    // The address is untouched and the typed text never split the label.
    expect(await addresses(page)).toEqual(["person/pk-morgan"]);
    const doc = (await read(page)).json as {
      content?: {
        content?: { type: string; attrs?: Record<string, string> }[];
      }[];
    };
    const tile = doc.content?.[0]?.content?.find((n) => n.type === "spikeTile");
    expect(tile?.attrs?.label).toBe("Morgan");
  });

  test("arrow traversal never lands inside a tile", async ({ page }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();
    await page.keyboard.press("End");
    await page.keyboard.type("x");

    // Walk left across the whole document. Whatever the caret does at the
    // boundaries, it must never split the tile: its address stays intact and
    // the tile count never changes.
    for (let i = 0; i < 4; i++) await page.keyboard.press("ArrowLeft");

    expect(await tileCount(page)).toBe(1);
    expect(await addresses(page)).toEqual(["person/pk-morgan"]);
  });

  test("backspace from just after a tile removes the whole tile", async ({
    page,
  }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();
    expect(await tileCount(page)).toBe(1);

    await page.keyboard.press("End");
    await page.keyboard.press("Backspace");

    expect(await tileCount(page)).toBe(0);
  });
});
