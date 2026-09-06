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
function rename(page: Page, id: string, label: string) {
  return page.evaluate(
    ({ id, label }) => window.__SPIKE_RENAME__?.({ kind: "person", id }, label),
    { id, label },
  );
}

function resetFaces(page: Page) {
  return page.evaluate(() => window.__SPIKE_RESET_FACES__?.());
}

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
  // The keyboard route to the leading boundary WORKS with the production node
  // view, in both engines. An earlier harness that rendered the tile as static
  // markup failed this — so the node view is not merely a nicer rendering, it
  // repaired a caret boundary. Both Home and ArrowLeft are covered because
  // keyboard routes must not diverge.
  test("Home then typing lands before a leading tile", async ({ page }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    await page.keyboard.press("Home");
    await page.keyboard.type("Hey ");

    const body = await text(page);
    expect(body.indexOf("Hey")).toBeLessThan(
      body.indexOf("buzz://person/pk-morgan"),
    );
  });

  // EXPECTED FAILURE, and now the only known caret gap. A pointer click on the
  // left edge of a leading tile lands after it, in Chromium and WebKit alike —
  // the documented browser defect (ProseMirror discuss #2538, #4502; Lexical
  // #6916). The keyboard routes above already work, so the boundary layer only
  // owes the pointer case. Remove `test.fail()` when it lands.
  test("clicking a leading tile's left edge lands before it", async ({
    page,
  }) => {
    test.fail();
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    const box = await page.locator(".inline-tile").first().boundingBox();
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
    const box = await page.locator(".inline-tile").first().boundingBox();
    if (!box) throw new Error("Tile is not laid out");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.type("ZZ");

    // The address is untouched and the typed text never split the label.
    expect(await addresses(page)).toEqual(["person/pk-morgan"]);
    // The rendered face is intact: typing did not split the label.
    await expect(page.locator(".inline-tile-label")).toHaveText("@Morgan");
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

test.describe("the address is the binding, not the name", () => {
  /**
   * The test that justifies replacing the current mention model.
   *
   * Two identities that share a display name, both inserted, then one renamed.
   * Both addresses survive intact and no identity ever appears in visible text.
   * The current client cannot do this: a same-name collision there forces a
   * 64-character key into the label, and a rename unbinds the recipient.
   */
  test("two same-name identities stay distinct across a rename", async ({
    page,
  }) => {
    await openSpike(page);

    // Give both identities the SAME display name, as two real teammates might.
    await rename(page, "pk-morgan", "Morgan");
    await rename(page, "pk-alex", "Morgan");

    await page.getByTestId("insert-morgan").click();
    await page.keyboard.type(" and ");
    await page.getByTestId("insert-alex").click();

    // Both tiles read "Morgan" and are still different references.
    await expect(page.locator(".inline-tile-label")).toHaveText([
      "@Morgan",
      "@Morgan",
    ]);
    expect(await addresses(page)).toEqual([
      "person/pk-morgan",
      "person/pk-alex",
    ]);

    // Rename one. The other is untouched, and neither address moved.
    await rename(page, "pk-alex", "Alex");
    await expect(page.locator(".inline-tile-label")).toHaveText([
      "@Morgan",
      "@Alex",
    ]);
    expect(await addresses(page)).toEqual([
      "person/pk-morgan",
      "person/pk-alex",
    ]);

    // No identity leaked into what a person reads.
    const visible = await page.getByTestId("spike-editor").innerText();
    expect(visible).not.toContain("pk-morgan");
    expect(visible).not.toContain("pk-alex");
  });

  /**
   * A rename must not edit the draft. If the face lived in the document, every
   * profile update would dirty an unsent message and enter undo history.
   */
  test("a rename does not alter the document", async ({ page }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();
    await page.keyboard.type(" please look");

    const before = await read(page);
    await rename(page, "pk-morgan", "Morgan Mulvaney");
    await expect(page.locator(".inline-tile-label")).toHaveText(
      "@Morgan Mulvaney",
    );

    const after = await read(page);
    expect(after.json).toEqual(before.json);
    expect(after.text).toBe(before.text);
  });

  /**
   * Undo after a rename must restore the person's text, not a stale name. This
   * is the concrete consequence of keeping the face out of the document.
   */
  test("undo is unaffected by a rename", async ({ page }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();
    await page.keyboard.type(" hello");
    await rename(page, "pk-morgan", "Morgan Mulvaney");

    // Undo is Meta+z on macOS. Using the wrong modifier silently does nothing
    // and the test would pass for the wrong reason.
    // One undo step covers the typing; a second covers the tile insert. What
    // matters is that undo walks the document the person authored and is not
    // perturbed by the rename at all.
    await page.keyboard.press(
      process.platform === "darwin" ? "Meta+z" : "Control+z",
    );
    expect(await text(page)).not.toContain("hello");
  });

  /**
   * A face resolved in one community must not survive into another. This is the
   * leak the desktop client's community-reset inventory exists to prevent.
   */
  test("a name does not survive a community reset", async ({ page }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();
    await expect(page.locator(".inline-tile-label")).toHaveText("@Morgan");

    await resetFaces(page);

    // The tile survives; only its name is forgotten. It falls back to an
    // abbreviation, never the full identity, and announces itself as
    // unresolved rather than reading an abbreviation aloud.
    await expect(page.locator(".inline-tile")).toHaveCount(1);
    // The forgotten name is gone. (The stand-in is derived from the id, which
    // in this harness is short enough not to be abbreviated; the abbreviation
    // rule for real 64-character identities is covered in the unit tests.)
    const label = await page.locator(".inline-tile-label").innerText();
    expect(label).not.toContain("Morgan");
    await expect(page.locator(".inline-tile")).toHaveAttribute(
      "aria-label",
      "Unresolved person",
    );

    // The address is untouched: forgetting a name never loses a reference.
    expect(await addresses(page)).toEqual(["person/pk-morgan"]);
  });

  /**
   * The plain-text projection is the address. This is what a sent message
   * carries and what an agent reads, and it is why a reader that knows nothing
   * about tiles still receives something meaningful.
   */
  test("the text projection is the address, never the label", async ({
    page,
  }) => {
    await openSpike(page);
    await page.getByTestId("insert-morgan").click();

    const body = await text(page);
    expect(body).toContain("buzz://person/pk-morgan");
    expect(body).not.toContain("Morgan");
  });
});
