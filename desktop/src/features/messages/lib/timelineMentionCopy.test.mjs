import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { truncateInlineChipLabel } from "@/shared/ui/mentionChip";

import {
  buildTimelineClipboardFlavors,
  handleTimelineMentionCopy,
} from "./timelineMentionCopy.ts";

// The copy handler works off a live selection and a rendered clone, so it needs
// real Selection/Range/getComputedStyle. jsdom has no layout, so `innerText` is
// undefined there and the plain flavor falls back to `selection.toString()` —
// the HTML flavor, which is what carries identity, is unaffected.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
  });
});

after(() => dom.window.close());

const JOHN_SMITH_PUBKEY = "7c".repeat(32);

/** Render `bodyHtml`, select the whole thing, and run `fn` in that state. */
function withRenderedSelection(bodyHtml, fn) {
  const body = dom.window.document.createElement("div");
  body.className = "message-markdown";
  body.innerHTML = bodyHtml;
  dom.window.document.body.append(body);

  const selection = dom.window.getSelection();
  selection.removeAllRanges();
  const range = dom.window.document.createRange();
  range.selectNodeContents(body);
  selection.addRange(range);

  try {
    return fn(selection);
  } finally {
    selection.removeAllRanges();
    body.remove();
  }
}

/** Render `bodyHtml` into the document and copy the whole thing. */
function copyRenderedBody(bodyHtml) {
  return withRenderedSelection(bodyHtml, buildTimelineClipboardFlavors);
}

test("a whole mention chip copies with its sigil and identity", () => {
  const flavors = copyRenderedBody(
    `hey <span data-mention="" data-mention-pubkey="${JOHN_SMITH_PUBKEY}" ` +
      'data-mention-label="John Smith" class="mention-chip">John Smith' +
      "</span> fixed it",
  );

  assert.notEqual(flavors, null);
  assert.equal(flavors.html.includes("@John Smith"), true);
  assert.equal(
    flavors.html.includes(`data-mention-pubkey="${JOHN_SMITH_PUBKEY}"`),
    true,
  );
});

test("a chip rendered past the length cap still copies its identity", () => {
  // A long channel name renders ellipsized while `data-channel-label` declares
  // it whole, so a *fully* selected chip's text is not its label. Classifying
  // that as a partial selection strips the identity, leaves `restored` false,
  // and drops the entire copy to the browser's default — dead, sigil-less text
  // with nothing to signal the failure.
  const label = `release-${"x".repeat(80)}-notes`;
  const rendered = truncateInlineChipLabel(label);
  assert.notEqual(rendered, label, "fixture must exceed the chip cap");

  const flavors = copyRenderedBody(
    `see <span data-channel-link="" data-channel-label="${label}" ` +
      `class="mention-chip">${rendered}</span> for details`,
  );

  assert.notEqual(flavors, null, "copy must not fall through to the default");
  assert.equal(flavors.html.includes(`#${label}`), true);
});

test("a partially selected chip loses its identity and gains no sigil", () => {
  // The guard this shares with the paste side: a fragment must never become a
  // mention the user did not copy.
  const flavors = copyRenderedBody(
    `<span data-mention="" data-mention-pubkey="${JOHN_SMITH_PUBKEY}" ` +
      'data-mention-label="John Smith" class="mention-chip">John</span>',
  );

  assert.equal(flavors, null);
});

const WHOLE_CHIP_BODY =
  `hey <span data-mention="" data-mention-pubkey="${JOHN_SMITH_PUBKEY}" ` +
  'data-mention-label="John Smith" class="mention-chip">John Smith' +
  "</span> fixed it";

test("the copy handler claims the event and writes both flavors", () => {
  withRenderedSelection(WHOLE_CHIP_BODY, () => {
    const written = new Map();
    let prevented = false;

    handleTimelineMentionCopy({
      defaultPrevented: false,
      preventDefault: () => {
        prevented = true;
      },
      clipboardData: {
        setData: (type, value) => written.set(type, value),
      },
    });

    assert.equal(prevented, true);
    assert.equal(written.get("text/html").includes("@John Smith"), true);
    // jsdom has no layout, so the plain flavor is the selection's own text —
    // sigil-less, but present.
    assert.equal(written.get("text/plain").includes("John Smith"), true);
  });
});

test("a copy whose clipboard data the browser withheld stays a no-op", () => {
  // preventDefault() ahead of the clipboardData guard would suppress the
  // default copy and then throw on setData — an empty clipboard. The handler
  // must decline before touching the event.
  withRenderedSelection(WHOLE_CHIP_BODY, () => {
    let prevented = false;

    handleTimelineMentionCopy({
      defaultPrevented: false,
      preventDefault: () => {
        prevented = true;
      },
      clipboardData: null,
    });

    assert.equal(prevented, false);
  });
});

test("copy inlines a blockified chip but preserves its block ancestor", () => {
  // jsdom has no innerText layout. Observe the clone at the actual read seam;
  // browser clipboard journeys separately assert the resulting plain text.
  const prototype = dom.window.HTMLElement.prototype;
  const previous = Object.getOwnPropertyDescriptor(prototype, "innerText");
  let observed = false;
  Object.defineProperty(prototype, "innerText", {
    configurable: true,
    get() {
      const chip = this.querySelector(".mention-chip");
      assert.equal(chip.style.display, "inline");
      assert.equal(chip.parentElement.style.display, "inline");
      assert.equal(chip.closest("p").style.display, "block");
      observed = true;
      return this.textContent;
    },
  });
  try {
    withRenderedSelection(
      '<p style="display:block">hey <span style="display:inline-flex">' +
        `<span data-mention="" data-mention-pubkey="${JOHN_SMITH_PUBKEY}" ` +
        'data-mention-label="John Smith" class="mention-chip" style="display:block">' +
        "John Smith</span></span> fixed it</p><p>Next paragraph</p>",
      (selection) => {
        const flavors = buildTimelineClipboardFlavors(selection);
        assert.ok(flavors);
        assert.equal(observed, true);
        assert.equal(
          document.querySelector(".mention-chip").style.display,
          "block",
          "the visible source must not be modified",
        );
      },
    );
  } finally {
    if (previous) Object.defineProperty(prototype, "innerText", previous);
    else delete prototype.innerText;
  }
});

test("copy expands compact key text but preserves the exact label and identity", () => {
  const label = `Scout (${JOHN_SMITH_PUBKEY}) 2`;
  const flavors = copyRenderedBody(
    `<span data-mention="" data-mention-pubkey="${JOHN_SMITH_PUBKEY}" ` +
      `data-mention-label="${label}" class="mention-chip">` +
      '<span class="inline-chip-leading-fragment">Scout</span> (7c7c7c7c…7c7c) 2</span>',
  );
  assert.ok(flavors);
  assert.ok(flavors.html.includes(`@${label}`));
  assert.ok(
    flavors.html.includes(`data-mention-pubkey="${JOHN_SMITH_PUBKEY}"`),
  );
  assert.ok(!flavors.html.includes("…"));
});
