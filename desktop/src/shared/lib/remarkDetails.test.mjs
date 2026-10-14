import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { renderCachedMarkdown } from "../ui/markdown/nodeCache.ts";
import remarkDetails, {
  MAX_DETAILS_DEPTH,
  normalizeMarkerLine,
  prepareDetailsBlocks,
} from "./remarkDetails.ts";

// Runs the production parser (nodeCache.ts); `details` and `summary` stand in
// for the MarkdownDetails components and expose the section key.
const components = {
  details: ({ children, ...props }) =>
    React.createElement(
      "section",
      { "data-key": props["data-details-key"] },
      children,
    ),
  summary: ({ children }) => React.createElement("h6", null, children),
};

function render(content) {
  return renderToStaticMarkup(
    renderCachedMarkdown({
      components,
      content,
      variant: "details-test",
    }),
  );
}

test("remarkDetails: folds a section whose closing marker follows a list", () => {
  const html = render(
    "Board\n:::details Waiting (2)\n- one\n- two\n:::\nAfter",
  );
  assert.equal(
    html,
    '<p>Board</p>\n<section data-key="0:Waiting (2)"><h6>Waiting (2)</h6>' +
      "<ul>\n<li>one</li>\n<li>two</li>\n</ul></section>\n<p>After</p>",
  );
});

test("remarkDetails: numbers repeated titles so keys stay distinct", () => {
  const html = render(":::details A\none\n:::\n\n:::details A\ntwo\n:::");
  assert.match(html, /data-key="0:A"/);
  assert.match(html, /data-key="1:A"/);
});

test("remarkDetails: nests sections", () => {
  const html = render(":::details Outer\n:::details Inner\nx\n:::\n:::");
  assert.match(
    html,
    /<section data-key="0:Outer"><h6>Outer<\/h6><section data-key="0:Inner"><h6>Inner<\/h6><p>x<\/p><\/section><\/section>/,
  );
});

test("remarkDetails: leaves an unclosed marker as text", () => {
  const html = render(":::details Never closed\nbody");
  assert.doesNotMatch(html, /<section/);
  assert.match(html, /:::details Never closed/);
});

test("remarkDetails: ignores markers inside fenced code", () => {
  const content = "```\n:::details Not a section\n:::\n```";
  assert.equal(prepareDetailsBlocks(content), content);
  assert.doesNotMatch(render(content), /<section/);
});

test("remarkDetails: ignores quoted markers and indented code", () => {
  assert.doesNotMatch(render("> :::details Quoted\n> x\n> :::"), /<section/);
  const code = "Code:\n\n    :::details Not a section\n    x\n    :::";
  assert.equal(prepareDetailsBlocks(code), code);
  assert.doesNotMatch(render(code), /<section/);
  for (const listFence of [
    "- item\n\n    ```\n    :::details Not a section\n    :::\n    ```",
    "- item\n\n    ```\n  :::details Not a section\n  secret\n  :::\n    ```",
  ]) {
    assert.equal(prepareDetailsBlocks(listFence), listFence);
    assert.doesNotMatch(render(listFence), /<section/);
  }
});

test("remarkDetails: accepts markers a composer paste indented into a list item", () => {
  // What the composer sends after pasting a section that ends in a list: the
  // closer becomes a continuation line of the last item.
  const html = render(
    ":::details **Fleet working** (2)\n\n- model routing\n" +
      "- OpenClaw upgrade\n  :::\n\n- x\n  :::details Next\nbody\n:::",
  );
  assert.equal(
    html,
    '<section data-key="0:Fleet working (2)"><h6><strong>Fleet working</strong> (2)</h6>' +
      "<ul>\n<li>model routing</li>\n<li>OpenClaw upgrade</li>\n</ul></section>\n" +
      '<ul>\n<li>x</li>\n</ul>\n<section data-key="0:Next"><h6>Next</h6><p>body</p></section>',
  );
});

test("remarkDetails: accepts no-break spaces the composer sent around a bold title", () => {
  // Bytes a user's composer sent: U+00A0 after `:::details` and
  // after the closing `**`, items as escaped-dash paragraphs.
  const html = render(
    ":::details\u00a0**Fleet working**\u00a0(2)\n\n\\- model routing\n\n" +
      "\\- OpenClaw upgrade\n\n:::",
  );
  assert.match(
    html,
    /^<section data-key="0:Fleet working\u00a0\(2\)"><h6><strong>Fleet working<\/strong>\u00a0\(2\)<\/h6>/,
  );
  assert.match(
    html,
    /<p>- model routing<\/p><p>- OpenClaw upgrade<\/p><\/section>$/,
  );
  assert.doesNotMatch(html, /:::/);
});

test("normalizeMarkerLine: turns no-break spaces in a marker into spaces", () => {
  assert.equal(
    normalizeMarkerLine(":::details\u00a0##\u00a0Title"),
    ":::details ## Title",
  );
  assert.equal(
    normalizeMarkerLine("**:::details\u00a0X**\u00a0"),
    ":::details **X**",
  );
  assert.equal(normalizeMarkerLine(":::\u00a0"), ":::");
  assert.equal(normalizeMarkerLine(":::details\u00a0"), null);
});

test("prepareDetailsBlocks: leaves content without markers untouched", () => {
  const content = "plain ::: text\n:::\n";
  assert.equal(prepareDetailsBlocks(content), content);
});

test("remarkDetails: keeps title formatting but flattens links", () => {
  const html = render(":::details [docs](https://example.com) **now**\nx\n:::");
  assert.match(html, /<h6>docs <strong>now<\/strong><\/h6>/);
  assert.doesNotMatch(html, /<a /);
  assert.match(html, /data-key="0:docs now"/);
});

test("remarkDetails: a heading title renders the summary inside a heading", () => {
  const html = render(":::details ## Waiting (2)\n- one\n:::");
  assert.match(
    html,
    /<section data-key="0:Waiting \(2\)"><h2><h6>Waiting \(2\)<\/h6><\/h2>/,
  );
});

test("remarkDetails: accepts marker lines the composer wrapped in bold", () => {
  // What the desktop composer sends with bold switched on.
  const html = render(
    "**:::details Bold (2)**\n**- one**\n**- two**\n**:::**\nAfter",
  );
  assert.match(
    html,
    /<section data-key="0:Bold \(2\)"><h6><strong>Bold \(2\)<\/strong><\/h6>/,
  );
  assert.match(html, /<p>After<\/p>/);
  assert.doesNotMatch(html, /:::/);
});

test("normalizeMarkerLine: moves a wrapper into the title, after a heading", () => {
  assert.equal(
    normalizeMarkerLine("**:::details ## X**"),
    ":::details ## **X**",
  );
  assert.equal(normalizeMarkerLine("_:::_"), ":::");
  assert.equal(normalizeMarkerLine("**text**"), null);
});

test("remarkDetails: markers past the depth limit stay text", () => {
  const depth = MAX_DETAILS_DEPTH + 1;
  const content =
    Array.from({ length: depth }, (_, i) => `:::details L${i}`).join("\n") +
    "\nx\n" +
    Array.from({ length: depth }, () => ":::").join("\n");
  const html = render(content);
  assert.equal(html.match(/<section/g)?.length, MAX_DETAILS_DEPTH);
  assert.match(html, new RegExp(`:::details L${MAX_DETAILS_DEPTH}`));
});

test("remarkDetails: survives hostile nesting near the message size limit", () => {
  const content = `${":::details x\n".repeat(3000)}${":::\n".repeat(3000)}`;
  assert.ok(content.length < 64 * 1024);
  const html = render(content);
  assert.equal(html.match(/<section/g)?.length, MAX_DETAILS_DEPTH);
});

test("remarkDetails: an info-string line does not close a fence", () => {
  const content = "```js\n``` trailing\n:::details Inside code\nx\n:::\n```";
  assert.equal(prepareDetailsBlocks(content), content);
  assert.doesNotMatch(render(content), /<section/);
});

test("remarkDetails: recognises markers with CRLF line endings", () => {
  const html = render("Board\r\n:::details A\r\nx\r\n:::\r\nAfter");
  assert.match(html, /<section data-key="0:A"><h6>A<\/h6><p>x<\/p><\/section>/);
});

test("remarkDetails: stays linear on blank-separated hostile nesting", () => {
  const levels = 3000;
  const content = `${":::details x\n\n".repeat(levels)}${":::\n\n".repeat(levels)}`;
  assert.ok(content.length < 64 * 1024);
  assert.equal(render(content).match(/<section/g)?.length, MAX_DETAILS_DEPTH);

  // Past the message size limit, so only linear work finishes in time: the
  // quadratic unwind this guards against would copy ~2 * 10^10 nodes here.
  const paragraph = (value) => ({
    type: "paragraph",
    children: [{ type: "text", value }],
  });
  const deep = 200_000;
  const tree = {
    type: "root",
    children: [
      ...Array.from({ length: deep }, () => paragraph(":::details x")),
      ...Array.from({ length: deep }, () => paragraph(":::")),
    ],
  };
  remarkDetails()(tree);
  assert.equal(tree.children.length, 1);
});

test("remarkDetails: falls back to a title when the title has no text", () => {
  const html = render(":::details [](https://example.com)\nsecret\n:::");
  assert.match(html, /<h6>Details<\/h6><p>secret<\/p>/);
});

test("remarkDetails: a section inside a block spoiler still folds", () => {
  const html = render("||\n:::details X\nbody\n:::\n||");
  assert.match(
    html,
    /<section data-key="0:X"><h6>X<\/h6><p>body<\/p><\/section>/,
  );
  assert.doesNotMatch(html, /:::/);
});
