import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { renderCachedMarkdown } from "../ui/markdown/nodeCache.ts";
import { prepareDetailsBlocks } from "./remarkDetails.ts";

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

test("remarkDetails: keeps inline markdown in the summary", () => {
  const html = render(":::details **Private** channels\nbody\n:::");
  assert.match(html, /<h6><strong>Private<\/strong> channels<\/h6>/);
  assert.match(html, /data-key="0:Private channels"/);
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

test("remarkDetails: ignores indented or quoted markers", () => {
  assert.doesNotMatch(render("  :::details Indented\nx\n  :::"), /<section/);
  assert.doesNotMatch(render("> :::details Quoted\n> x\n> :::"), /<section/);
});

test("prepareDetailsBlocks: leaves content without markers untouched", () => {
  const content = "plain ::: text\n:::\n";
  assert.equal(prepareDetailsBlocks(content), content);
});
