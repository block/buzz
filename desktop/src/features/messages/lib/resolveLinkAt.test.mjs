import assert from "node:assert/strict";
import test from "node:test";

import { Schema } from "@tiptap/pm/model";
import { EditorState } from "@tiptap/pm/state";

import { resolveLinkAt } from "./resolveLinkAt.ts";

// Minimal schema mirroring the editor's relevant pieces: a link mark with an
// href plus a bold mark, so a single link can be split across text nodes.
const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    paragraph: { group: "block", content: "inline*" },
    text: { group: "inline" },
  },
  marks: {
    link: { attrs: { href: {} }, inclusive: false },
    bold: {},
  },
});

const link = (href) => schema.marks.link.create({ href });
const bold = schema.marks.bold.create();

function stateFromParagraph(nodes) {
  const doc = schema.node("doc", null, [schema.node("paragraph", null, nodes)]);
  return EditorState.create({ doc });
}

// ── resolveLinkAt ─────────────────────────────────────────────────────

test("resolves a plain single-node link", () => {
  const href = "https://example.com";
  const state = stateFromParagraph([schema.text("click here", [link(href)])]);

  // pos 1 is the start of the paragraph content (inside the link text).
  const info = resolveLinkAt(state, 3);
  assert.ok(info);
  assert.equal(info.href, href);
  assert.equal(info.text, "click here");
  assert.equal(info.from, 1);
  assert.equal(info.to, 1 + "click here".length);
});

test("resolves a link split across mixed-formatting text nodes", () => {
  const href = "https://example.com";
  // "see " + bold "this" + " link" — three text nodes, one visual link.
  const state = stateFromParagraph([
    schema.text("see ", [link(href)]),
    schema.text("this", [link(href), bold]),
    schema.text(" link", [link(href)]),
  ]);

  const full = "see this link";
  // Click lands inside the bold fragment in the middle.
  const info = resolveLinkAt(state, 7);
  assert.ok(info);
  assert.equal(info.href, href);
  // The whole link must be recovered, not just the bold fragment.
  assert.equal(info.text, full);
  assert.equal(info.from, 1);
  assert.equal(info.to, 1 + full.length);
});

test("does not extend across an adjacent link with a different href", () => {
  const a = "https://a.com";
  const b = "https://b.com";
  const state = stateFromParagraph([
    schema.text("alpha", [link(a)]),
    schema.text("beta", [link(b)]),
  ]);

  const info = resolveLinkAt(state, 3);
  assert.ok(info);
  assert.equal(info.href, a);
  assert.equal(info.text, "alpha");
  assert.equal(info.from, 1);
  assert.equal(info.to, 1 + "alpha".length);
});

test("anchors on the left link at the seam between two adjacent links", () => {
  const a = "https://a.com";
  const b = "https://b.com";
  // "alpha" (link a) immediately followed by "beta" (link b). The seam sits
  // at pos 6 (paragraph start 1 + "alpha".length). Both child spans touch
  // that position; the caret there belongs to the character before it, so we
  // must resolve to link `a`, not whichever span the iteration visits last.
  const state = stateFromParagraph([
    schema.text("alpha", [link(a)]),
    schema.text("beta", [link(b)]),
  ]);

  const seam = 1 + "alpha".length;
  const info = resolveLinkAt(state, seam);
  assert.ok(info);
  assert.equal(info.href, a);
  assert.equal(info.text, "alpha");
  assert.equal(info.from, 1);
  assert.equal(info.to, seam);
});

test("resolves a link when the caret sits at its very start", () => {
  const href = "https://example.com";
  // Plain text, then the link — caret at the link's left edge has no link
  // mark on the character *before* it, exercising the `pos + 1` fallback.
  const state = stateFromParagraph([
    schema.text("go "),
    schema.text("here", [link(href)]),
  ]);

  const linkStart = 1 + "go ".length;
  const info = resolveLinkAt(state, linkStart);
  assert.ok(info);
  assert.equal(info.href, href);
  assert.equal(info.text, "here");
  assert.equal(info.from, linkStart);
  assert.equal(info.to, linkStart + "here".length);
});

test("returns null when position is not inside a link", () => {
  const state = stateFromParagraph([schema.text("no link here")]);
  assert.equal(resolveLinkAt(state, 3), null);
});
