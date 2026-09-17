import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { PlaygroundCard } from "@/features/playground/ui/PlaygroundCard";
import { parsePlaygroundCard } from "@/features/playground/lib/card";
import { createMarkdownComponents } from "../markdown.tsx";
import { getReactNodeText } from "./utils.ts";
import { renderCachedMarkdown } from "./nodeCache.ts";
import { MarkdownFencedCode, MarkdownFencedPre } from "./fencedBlocks.tsx";

const STITCH_JSON =
  '{"hula":"playground","v":1,"name":"Mock playground","url":"https://www.google.com","pin":"123456","sid":"mock_google_1"}';

/** Exact #hula event body: lead-in, fence opener, blank line, JSON, blank line, close. */
const STITCH_BODY = `Playground card:
\`\`\`playground

${STITCH_JSON}

\`\`\``;

const COMPACT_JSON =
  '{"hula":"playground","v":1,"name":"Compact playground","url":"https://example.com","pin":"99","sid":"compact_1"}';

const HULA_PORT_HOLE_JSON =
  '{"hula":"playground","v":1,"name":"hula-port-hole","url":"https://hula-port-hole.hulapreview.com","sid":"oc-preview-hula-port-hole","stack":"main","expires":1787564786}';

function isPlaygroundCardElement(node) {
  return (
    React.isValidElement(node) &&
    node.type === PlaygroundCard &&
    node.props?.card?.hula === "playground"
  );
}

/** react-markdown leaves `code` as MarkdownFencedCode until render; invoke it. */
function resolveFencedCode(node) {
  if (!React.isValidElement(node)) return node;
  if (node.type === MarkdownFencedCode) {
    return MarkdownFencedCode(node.props);
  }
  if (node.type === MarkdownFencedPre) {
    return MarkdownFencedPre(node.props);
  }
  if (typeof node.type === "string" && node.type === "pre") {
    const children = React.Children.map(node.props.children, resolveFencedCode);
    // Mimic createMarkdownComponents pre wrapper.
    return MarkdownFencedPre({
      interactive: true,
      children,
    });
  }
  if (node.props?.children != null) {
    return React.cloneElement(node, {
      children: React.Children.map(node.props.children, resolveFencedCode),
    });
  }
  return node;
}

function collectPlaygroundCards(node, out = []) {
  if (node == null || typeof node === "boolean") return out;
  if (Array.isArray(node)) {
    for (const child of node) collectPlaygroundCards(child, out);
    return out;
  }
  if (isPlaygroundCardElement(node)) {
    out.push(node.props.card);
    return out;
  }
  if (React.isValidElement(node)) {
    collectPlaygroundCards(node.props?.children, out);
  }
  return out;
}

function cardsFromMarkdown(content, variant) {
  const tree = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content,
    variant,
  });
  return collectPlaygroundCards(resolveFencedCode(tree));
}

test("getReactNodeText joins fence children without the String(array) commas", () => {
  const nodes = ["\n", `${STITCH_JSON}\n`];
  assert.equal(String(nodes), `\n,${STITCH_JSON}\n`);
  assert.equal(getReactNodeText(nodes), `\n${STITCH_JSON}\n`);
});

test("Stitch playground fence with a blank line after the opener renders a card", () => {
  const cards = cardsFromMarkdown(STITCH_BODY, "playground-stitch-blank-line");
  assert.equal(cards.length, 1);
  assert.equal(cards[0].name, "Mock playground");
  assert.equal(cards[0].url, "https://www.google.com");
  assert.equal(cards[0].pin, "123456");
  assert.equal(cards[0].sid, "mock_google_1");
});

test("invalid playground fence shows the source instead of swallowing", () => {
  const tree = resolveFencedCode(
    renderCachedMarkdown({
      components: createMarkdownComponents(true, false),
      content: "```playground\nnot valid json {{\n```",
      variant: "playground-garbage-fence",
    }),
  );
  assert.equal(collectPlaygroundCards(tree).length, 0);
  assert.match(getReactNodeText(tree), /not valid json \{\{/);
});

test("compact one-line playground fence still renders a card", () => {
  const cards = cardsFromMarkdown(
    `\`\`\`playground\n${COMPACT_JSON}\n\`\`\``,
    "playground-compact-fence",
  );
  assert.equal(cards.length, 1);
  assert.equal(cards[0].name, "Compact playground");
  assert.equal(cards[0].url, "https://example.com");
  assert.equal(cards[0].pin, "99");
});

test("hula-port-hole playground fence renders the playground card", () => {
  const cards = cardsFromMarkdown(
    `\`\`\`playground\n${HULA_PORT_HOLE_JSON}\n\`\`\``,
    "playground-hula-port-hole-fence",
  );
  assert.equal(cards.length, 1);
  assert.deepEqual(cards[0], {
    hula: "playground",
    v: 1,
    name: "hula-port-hole",
    url: "https://hula-port-hole.hulapreview.com",
    sid: "oc-preview-hula-port-hole",
    stack: "main",
    expires: 1787564786,
  });
});

test("hula-port-hole json fence renders the playground card", () => {
  const cards = cardsFromMarkdown(
    `\`\`\`json\n${HULA_PORT_HOLE_JSON}\n\`\`\``,
    "playground-hula-port-hole-json-fence",
  );
  assert.equal(cards.length, 1);
  assert.equal(cards[0].name, "hula-port-hole");
  assert.equal(cards[0].v, 1);
});

test("hula-port-hole unlabeled fence renders the playground card", () => {
  const cards = cardsFromMarkdown(
    `\`\`\`\n${HULA_PORT_HOLE_JSON}\n\`\`\``,
    "playground-hula-port-hole-unlabeled-fence",
  );
  assert.equal(cards.length, 1);
  assert.equal(cards[0].name, "hula-port-hole");
});

test("MarkdownFencedCode recognizes hula-port-hole payload for playground/json/bare languages", () => {
  for (const language of ["playground", "json", ""]) {
    const className = language ? `language-${language}` : undefined;
    const element = MarkdownFencedCode({
      className,
      children: `${HULA_PORT_HOLE_JSON}\n`,
    });
    assert.equal(isPlaygroundCardElement(element), true, language || "(none)");
    assert.equal(element.props.card.name, "hula-port-hole");
  }
});

test("hula-port-hole bare JSON is recognized by parse path used for message bodies", () => {
  const card = parsePlaygroundCard(HULA_PORT_HOLE_JSON);
  assert.deepEqual(card, {
    hula: "playground",
    v: 1,
    name: "hula-port-hole",
    url: "https://hula-port-hole.hulapreview.com",
    sid: "oc-preview-hula-port-hole",
    stack: "main",
    expires: 1787564786,
  });
});
