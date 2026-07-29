import assert from "node:assert/strict";
import test from "node:test";

import { renderDevMarkdown } from "./devMarkdown.tsx";

function elements(nodes, type) {
  return nodes.filter(
    (node) => typeof node === "object" && node !== null && node.type === type,
  );
}

function textOf(node) {
  if (node === null || node === undefined) return "";
  if (typeof node === "string") return node;
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (typeof node === "object") return textOf(node.props?.children);
  return String(node);
}

test("plainText_staysOnePreWrapParagraph", () => {
  const nodes = renderDevMarkdown("hello\nworld");
  assert.equal(nodes.length, 1);
  assert.equal(nodes[0].type, "p");
  assert.equal(textOf(nodes[0]), "hello\nworld");
});

test("blankLine_splitsParagraphs", () => {
  const nodes = renderDevMarkdown("one\n\ntwo");
  assert.equal(elements(nodes, "p").length, 2);
});

test("fencedCode_rendersPreWithoutFenceLines", () => {
  const nodes = renderDevMarkdown("before\n```ts\nconst a = 1;\n```\nafter");
  const [pre] = elements(nodes, "pre");
  assert.ok(pre);
  assert.equal(textOf(pre), "const a = 1;");
  assert.equal(elements(nodes, "p").length, 2);
});

test("unterminatedFence_capturesRestOfMessage", () => {
  const nodes = renderDevMarkdown("```\nline1\nline2");
  const [pre] = elements(nodes, "pre");
  assert.equal(textOf(pre), "line1\nline2");
});

test("heading_rendersBoldWithoutHashes", () => {
  const nodes = renderDevMarkdown("## Findings");
  assert.equal(nodes.length, 1);
  assert.equal(textOf(nodes[0]), "Findings");
  assert.ok(nodes[0].props.className.includes("font-bold"));
});

test("bulletList_rendersMarkerAndInlineContent", () => {
  const nodes = renderDevMarkdown("- **Saved rule state** — conforms");
  assert.equal(nodes.length, 1);
  const [marker, body] = nodes[0].props.children;
  assert.equal(textOf(marker), "•");
  assert.equal(textOf(body), "Saved rule state — conforms");
});

test("numberedList_keepsNumbers", () => {
  const nodes = renderDevMarkdown("1. first\n2. second");
  assert.equal(nodes.length, 2);
  assert.equal(textOf(nodes[0].props.children[0]), "1.");
  assert.equal(textOf(nodes[1].props.children[0]), "2.");
});

test("nestedListItem_indents", () => {
  const nodes = renderDevMarkdown("- top\n  - nested");
  assert.equal(nodes[1].props.style.paddingLeft, "2ch");
});

test("blockquote_groupsConsecutiveLines", () => {
  const nodes = renderDevMarkdown("> one\n> two\nplain");
  const [quote] = elements(nodes, "blockquote");
  assert.equal(textOf(quote), "one\ntwo");
  assert.equal(elements(nodes, "p").length, 1);
});

test("horizontalRule_rendersDivider", () => {
  const nodes = renderDevMarkdown("above\n---\nbelow");
  const divider = nodes.find(
    (node) =>
      typeof node === "object" && node.props?.className?.includes("border-t"),
  );
  assert.ok(divider);
});

test("hyphenListItem_isNotAHorizontalRule", () => {
  const nodes = renderDevMarkdown("- item");
  assert.equal(textOf(nodes[0].props.children[0]), "•");
});
