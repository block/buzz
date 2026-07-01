import assert from "node:assert/strict";
import test from "node:test";

// These are copied here to avoid importing from .ts files that depend on
// React (which isn't resolvable outside the bundler). Same pattern as
// useMediaUpload.test.mjs inlining shortHash.

function shallowArrayEqual(a, b) {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

// Minimal React.isValidElement check — real React checks $$typeof
const REACT_ELEMENT_TYPE =
  Symbol.for("react.transitional.element") ?? Symbol.for("react.element");

function isValidElement(obj) {
  return (
    typeof obj === "object" &&
    obj !== null &&
    obj.$$typeof === REACT_ELEMENT_TYPE
  );
}

function fakeElement(type, props = {}) {
  return { $$typeof: REACT_ELEMENT_TYPE, type, props, key: null };
}

function isBlockMedia(child) {
  return isValidElement(child) && child.props?.["data-block-media"] != null;
}

function classifyChildren(childArray) {
  const imageChildren = childArray.filter(isBlockMedia);
  const nonImageChildren = childArray.filter(
    (child) =>
      !isBlockMedia(child) &&
      !(typeof child === "string" && child.trim() === "") &&
      !(isValidElement(child) && child.type === "br"),
  );
  return { imageChildren, nonImageChildren };
}

function isImageOnlyParagraph(childArray) {
  const { imageChildren, nonImageChildren } = classifyChildren(childArray);
  return imageChildren.length >= 2 && nonImageChildren.length === 0;
}

function hasBlockMedia(childArray) {
  const { imageChildren, nonImageChildren } = classifyChildren(childArray);
  return imageChildren.length >= 1 && nonImageChildren.length === 0;
}

function isHastElement(node) {
  return node && node.type === "element";
}

function isHastText(node) {
  return node && node.type === "text";
}

function isHastImageOnlyParagraph(node) {
  if (!isHastElement(node) || node.tagName !== "p") return false;
  const meaningful = node.children.filter(
    (child) =>
      !(isHastText(child) && child.value.trim() === "") &&
      !(isHastElement(child) && child.tagName === "br"),
  );
  return (
    meaningful.length >= 1 &&
    meaningful.every((child) => isHastElement(child) && child.tagName === "img")
  );
}

function rehypeImageGallery() {
  return (tree) => {
    const newChildren = [];
    let imageRun = [];

    function flushRun() {
      if (imageRun.length <= 1) {
        newChildren.push(...imageRun);
      } else {
        const allImages = [];
        for (const p of imageRun) {
          for (const child of p.children) {
            if (isHastElement(child) && child.tagName === "img") {
              allImages.push(child);
            }
          }
        }
        newChildren.push({
          type: "element",
          tagName: "p",
          properties: {},
          children: allImages,
        });
      }
      imageRun = [];
    }

    for (const child of tree.children) {
      if (isHastImageOnlyParagraph(child)) {
        imageRun.push(child);
        continue;
      }
      flushRun();
      newChildren.push(child);
    }
    flushRun();

    tree.children = newChildren;
  };
}

test("shallowArrayEqual: identical references return true", () => {
  const arr = ["a", "b"];
  assert.equal(shallowArrayEqual(arr, arr), true);
});

test("shallowArrayEqual: equal arrays return true", () => {
  assert.equal(shallowArrayEqual(["a", "b"], ["a", "b"]), true);
});

test("shallowArrayEqual: different values return false", () => {
  assert.equal(shallowArrayEqual(["a", "b"], ["a", "c"]), false);
});

test("shallowArrayEqual: different lengths return false", () => {
  assert.equal(shallowArrayEqual(["a"], ["a", "b"]), false);
});

test("shallowArrayEqual: both undefined return true", () => {
  assert.equal(shallowArrayEqual(undefined, undefined), true);
});

test("shallowArrayEqual: one undefined returns false", () => {
  assert.equal(shallowArrayEqual(["a"], undefined), false);
  assert.equal(shallowArrayEqual(undefined, ["a"]), false);
});

test("shallowArrayEqual: empty arrays return true", () => {
  assert.equal(shallowArrayEqual([], []), true);
});

test("classifyChildren: elements with data-block-media are image children", () => {
  const children = [fakeElement("span", { "data-block-media": "" })];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 1);
  assert.equal(nonImageChildren.length, 0);
});

test("classifyChildren: React component elements without data-block-media are non-image", () => {
  const LinkComponent = () => null;
  const children = [fakeElement(LinkComponent)];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 0);
  assert.equal(nonImageChildren.length, 1);
});

test("classifyChildren: plain HTML elements are non-image children", () => {
  const children = [fakeElement("span")];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 0);
  assert.equal(nonImageChildren.length, 1);
});

test("classifyChildren: text strings are non-image children", () => {
  const children = ["hello world"];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 0);
  assert.equal(nonImageChildren.length, 1);
});

test("classifyChildren: whitespace-only strings are excluded from both", () => {
  const children = ["  ", "\n"];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 0);
  assert.equal(nonImageChildren.length, 0);
});

test("classifyChildren: <br> elements are excluded from non-image", () => {
  const children = [fakeElement("br")];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 0);
  assert.equal(nonImageChildren.length, 0);
});

test("classifyChildren: mixed media, text, and br", () => {
  const children = [
    fakeElement("span", { "data-block-media": "" }),
    "some text",
    fakeElement("br"),
    fakeElement("span", { "data-block-media": "" }),
  ];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 2);
  assert.equal(nonImageChildren.length, 1); // "some text"
});

test("classifyChildren: media with only whitespace and br between them", () => {
  const children = [
    fakeElement("span", { "data-block-media": "" }),
    "  ",
    fakeElement("br"),
    fakeElement("span", { "data-block-media": "" }),
  ];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 2);
  assert.equal(nonImageChildren.length, 0);
});

test("isImageOnlyParagraph: two media with br returns true", () => {
  const media = { "data-block-media": "" };
  const children = [
    fakeElement("span", media),
    fakeElement("br"),
    fakeElement("span", media),
  ];
  assert.equal(isImageOnlyParagraph(children), true);
});

test("isImageOnlyParagraph: single media returns false (needs 2+)", () => {
  const children = [fakeElement("span", { "data-block-media": "" })];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("isImageOnlyParagraph: media with text returns false", () => {
  const media = { "data-block-media": "" };
  const children = [
    fakeElement("span", media),
    "caption text",
    fakeElement("span", media),
  ];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("isImageOnlyParagraph: no children returns false", () => {
  assert.equal(isImageOnlyParagraph([]), false);
});

test("isImageOnlyParagraph: three media returns true", () => {
  const media = { "data-block-media": "" };
  const children = [
    fakeElement("span", media),
    fakeElement("span", media),
    fakeElement("span", media),
  ];
  assert.equal(isImageOnlyParagraph(children), true);
});

test("isImageOnlyParagraph: plain HTML img tags without data-block-media are non-image", () => {
  const children = [fakeElement("img"), fakeElement("img")];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("isImageOnlyParagraph: non-media component + media is not image-only", () => {
  const LinkComponent = () => null;
  const media = { "data-block-media": "" };
  const children = [
    fakeElement(LinkComponent),
    fakeElement("span", media),
    fakeElement("span", media),
  ];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("hasBlockMedia: single media element returns true", () => {
  assert.equal(
    hasBlockMedia([fakeElement("span", { "data-block-media": "" })]),
    true,
  );
});

test("hasBlockMedia: two media returns true", () => {
  const media = { "data-block-media": "" };
  assert.equal(
    hasBlockMedia([fakeElement("span", media), fakeElement("span", media)]),
    true,
  );
});

test("hasBlockMedia: media with whitespace and br returns true", () => {
  assert.equal(
    hasBlockMedia([
      fakeElement("span", { "data-block-media": "" }),
      "  ",
      fakeElement("br"),
    ]),
    true,
  );
});

test("hasBlockMedia: no children returns false", () => {
  assert.equal(hasBlockMedia([]), false);
});

test("hasBlockMedia: text only returns false", () => {
  assert.equal(hasBlockMedia(["hello"]), false);
});

test("hasBlockMedia: media with text returns false", () => {
  assert.equal(
    hasBlockMedia([fakeElement("span", { "data-block-media": "" }), "caption"]),
    false,
  );
});

test("hasBlockMedia: plain HTML img without data-block-media returns false", () => {
  assert.equal(hasBlockMedia([fakeElement("img")]), false);
});

test("hasBlockMedia: React component without data-block-media returns false", () => {
  const LinkComponent = () => null;
  assert.equal(hasBlockMedia([fakeElement(LinkComponent)]), false);
});

function hastImg(src) {
  return { type: "element", tagName: "img", properties: { src }, children: [] };
}

function hastP(...children) {
  return { type: "element", tagName: "p", properties: {}, children };
}

function hastText(value) {
  return { type: "text", value };
}

test("rehypeImageGallery: merges two consecutive single-image paragraphs", () => {
  const tree = {
    type: "root",
    children: [hastP(hastImg("a.png")), hastP(hastImg("b.png"))],
  };
  rehypeImageGallery()(tree);
  assert.equal(tree.children.length, 1);
  assert.equal(tree.children[0].tagName, "p");
  assert.equal(tree.children[0].children.length, 2);
  assert.equal(tree.children[0].children[0].properties.src, "a.png");
  assert.equal(tree.children[0].children[1].properties.src, "b.png");
});

test("rehypeImageGallery: three consecutive images merge into one paragraph", () => {
  const tree = {
    type: "root",
    children: [
      hastP(hastImg("a.png")),
      hastP(hastImg("b.png")),
      hastP(hastImg("c.png")),
    ],
  };
  rehypeImageGallery()(tree);
  assert.equal(tree.children.length, 1);
  assert.equal(tree.children[0].children.length, 3);
});

test("rehypeImageGallery: single image paragraph is not grouped", () => {
  const tree = {
    type: "root",
    children: [hastP(hastImg("a.png"))],
  };
  rehypeImageGallery()(tree);
  assert.equal(tree.children.length, 1);
  // Still the original single-image paragraph
  assert.equal(tree.children[0].children.length, 1);
});

test("rehypeImageGallery: text paragraph breaks image run", () => {
  const tree = {
    type: "root",
    children: [
      hastP(hastImg("a.png")),
      hastP(hastText("hello")),
      hastP(hastImg("b.png")),
    ],
  };
  rehypeImageGallery()(tree);
  assert.equal(tree.children.length, 3);
  // Each stays separate — text paragraph broke the run
  assert.equal(tree.children[0].children[0].properties.src, "a.png");
  assert.equal(tree.children[1].children[0].value, "hello");
  assert.equal(tree.children[2].children[0].properties.src, "b.png");
});

test("rehypeImageGallery: ignores whitespace and br in image paragraphs", () => {
  const br = { type: "element", tagName: "br", properties: {}, children: [] };
  const tree = {
    type: "root",
    children: [
      hastP(hastImg("a.png"), hastText("  "), br),
      hastP(hastImg("b.png")),
    ],
  };
  rehypeImageGallery()(tree);
  assert.equal(tree.children.length, 1);
  assert.equal(tree.children[0].children.length, 2);
});

test("rehypeImageGallery: mixed content paragraph is not image-only", () => {
  const tree = {
    type: "root",
    children: [
      hastP(hastImg("a.png")),
      hastP(hastText("Look: "), hastImg("b.png")),
      hastP(hastImg("c.png")),
    ],
  };
  rehypeImageGallery()(tree);
  // Middle paragraph has text, so it breaks the run
  assert.equal(tree.children.length, 3);
});

// Regression test: react-markdown's `defaultUrlTransform` strips unknown
// schemes (returns `""`) before our `a` component override can see them,
// which would break copy → paste → click for `buzz://message?…` links
// end-to-end. We pass a custom `urlTransform` that delegates to the
// default for `buzz://message` and legacy `buzz://message` hrefs.
//
// This test renders real `<ReactMarkdown>` with the production transform
// and asserts the link href survives to the rendered DOM. Mirrors the
// `markdown.tsx` source — keep in sync if either changes.

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";

import { isMessageLink } from "../../features/messages/lib/messageLink.ts";
import remarkSpoilers from "../lib/remarkSpoilers.ts";

function messageLinkUrlTransform(value, key) {
  if (key === "href" && isMessageLink(value)) {
    return value;
  }
  return defaultUrlTransform(value);
}

function renderMarkdown(content) {
  return renderToStaticMarkup(
    React.createElement(
      ReactMarkdown,
      { urlTransform: messageLinkUrlTransform },
      content,
    ),
  );
}

test("messageLinkUrlTransform: preserves buzz://message href", () => {
  const html = renderMarkdown(
    "Click [here](buzz://message?channel=abc&id=xyz)",
  );
  // HTML-encoded `&` in attributes is fine — the browser decodes back to `&`.
  assert.match(html, /href="buzz:\/\/message\?channel=abc&(?:amp;)?id=xyz"/);
});

test("messageLinkUrlTransform: preserves buzz://message autolink href", () => {
  const html = renderMarkdown("<buzz://message?channel=abc&id=xyz>");
  assert.match(html, /href="buzz:\/\/message\?channel=abc&(?:amp;)?id=xyz"/);
});

test("messageLinkUrlTransform: preserves buzz://message href with thread", () => {
  const html = renderMarkdown(
    "[link](buzz://message?channel=c1&id=m1&thread=t1)",
  );
  assert.match(html, /href="buzz:\/\/message\?[^"]*thread=t1"/);
});

test("messageLinkUrlTransform: still strips javascript: scheme", () => {
  const html = renderMarkdown("[xss](javascript:alert(1))");
  // defaultUrlTransform replaces unsafe schemes with the empty string.
  assert.match(html, /href=""/);
  assert.doesNotMatch(html, /javascript:/);
});

test("messageLinkUrlTransform: passes http(s) through unchanged", () => {
  const html = renderMarkdown("[ext](https://example.com/path)");
  assert.match(html, /href="https:\/\/example\.com\/path"/);
});

test("messageLinkUrlTransform: preserves legacy buzz://message href", () => {
  const html = renderMarkdown(
    "Click [here](buzz://message?channel=abc&id=xyz)",
  );
  assert.match(html, /href="buzz:\/\/message\?channel=abc&(?:amp;)?id=xyz"/);
});

test("messageLinkUrlTransform: leaves non-message buzz:// schemes to default", () => {
  // `buzz://connect?relay=…` is handled by a different code path (Tauri
  // single-instance). The markdown renderer should let it pass through
  // defaultUrlTransform (which strips it) since it's not clickable in-app.
  const html = renderMarkdown(
    "[connect](buzz://connect?relay=wss://relay.example)",
  );
  assert.match(html, /href=""/);
});

test("remarkSpoilers: block delimiter spoilers expose a block prop to React", () => {
  let spoilerProps;
  renderToStaticMarkup(
    React.createElement(
      ReactMarkdown,
      {
        components: {
          spoiler: (props) => {
            spoilerProps = props;
            return React.createElement("div", null, props.children);
          },
        },
        remarkPlugins: [remarkSpoilers],
      },
      "||\n\nsecret\n\n||",
    ),
  );

  assert.equal(spoilerProps?.["data-block-spoiler"], "");
});

// `remark-gfm`'s autolinker only covers http(s)://, so bare `buzz://message`
// URLs in plain text never reach any rendering path without this plugin.
// The plugin emits a custom `message-link` HAST element which markdown.tsx
// renders as an inline pill. Tests operate on the mdast tree directly —
// the rendering side is a plain React component covered by app-level use.

import remarkMessageLinks from "../../features/messages/lib/remarkMessageLinks.ts";

function runPlugin(tree) {
  remarkMessageLinks()(tree);
  return tree;
}

function paragraph(...children) {
  return { type: "root", children: [{ type: "paragraph", children }] };
}

function text(value) {
  return { type: "text", value };
}

test("remarkMessageLinks: bare buzz://message URL is replaced", () => {
  const tree = runPlugin(paragraph(text("buzz://message?channel=c&id=m")));
  const para = tree.children[0];
  assert.equal(para.children.length, 1);
  assert.equal(para.children[0].type, "message-link");
  assert.equal(para.children[0].value, "buzz://message?channel=c&id=m");
  assert.equal(para.children[0].data.hName, "message-link");
});

test("remarkMessageLinks: legacy bare buzz://message URL is replaced", () => {
  const tree = runPlugin(paragraph(text("buzz://message?channel=c&id=m")));
  const para = tree.children[0];
  assert.equal(para.children.length, 1);
  assert.equal(para.children[0].type, "message-link");
  assert.equal(para.children[0].value, "buzz://message?channel=c&id=m");
});

test("remarkMessageLinks: mid-sentence URL splits surrounding text", () => {
  const tree = runPlugin(
    paragraph(text("see buzz://message?channel=c&id=m here")),
  );
  const kids = tree.children[0].children;
  assert.equal(kids.length, 3);
  assert.equal(kids[0].type, "text");
  assert.equal(kids[0].value, "see ");
  assert.equal(kids[1].type, "message-link");
  assert.equal(kids[2].type, "text");
  assert.equal(kids[2].value, " here");
});

test("remarkMessageLinks: two URLs in one text node both replaced", () => {
  const tree = runPlugin(
    paragraph(
      text(
        "first buzz://message?channel=a&id=1 then buzz://message?channel=b&id=2 done",
      ),
    ),
  );
  const kids = tree.children[0].children;
  const links = kids.filter((c) => c.type === "message-link");
  assert.equal(links.length, 2);
  assert.equal(links[0].value, "buzz://message?channel=a&id=1");
  assert.equal(links[1].value, "buzz://message?channel=b&id=2");
});

test("remarkMessageLinks: trailing sentence punctuation stays outside URL", () => {
  for (const punctuation of [".", ",", ";", ":", "!", "?"]) {
    const tree = runPlugin(
      paragraph(text(`see buzz://message?channel=c&id=m${punctuation}`)),
    );
    const kids = tree.children[0].children;

    assert.equal(kids.length, 3, punctuation);
    assert.equal(kids[0].value, "see ", punctuation);
    assert.equal(kids[1].type, "message-link", punctuation);
    assert.equal(kids[1].value, "buzz://message?channel=c&id=m", punctuation);
    assert.equal(kids[2].type, "text", punctuation);
    assert.equal(kids[2].value, punctuation, punctuation);
  }
});

test("remarkMessageLinks: URL inside parens keeps closing paren outside", () => {
  const tree = runPlugin(
    paragraph(text("see (buzz://message?channel=c&id=m) for details")),
  );
  const kids = tree.children[0].children;

  assert.equal(kids.length, 3);
  assert.equal(kids[0].value, "see (");
  assert.equal(kids[1].type, "message-link");
  assert.equal(kids[1].value, "buzz://message?channel=c&id=m");
  assert.equal(kids[2].type, "text");
  assert.equal(kids[2].value, ") for details");
});

test("remarkMessageLinks: URL without trailing punctuation matches end-to-end", () => {
  const value = "buzz://message?channel=c&id=m";
  const tree = runPlugin(paragraph(text(value)));
  const kids = tree.children[0].children;

  assert.equal(kids.length, 1);
  assert.equal(kids[0].type, "message-link");
  assert.equal(kids[0].value, value);
});

test("remarkMessageLinks: non-message buzz:// URLs are not matched", () => {
  const original = "buzz://connect?relay=wss://x.example";
  const tree = runPlugin(paragraph(text(original)));
  const kids = tree.children[0].children;
  assert.equal(kids.length, 1);
  assert.equal(kids[0].type, "text");
  assert.equal(kids[0].value, original);
});

test("remarkMessageLinks: text inside inlineCode is left alone", () => {
  // The shared factory's tree walker descends into all non-text nodes; an
  // `inlineCode` node has its URL stored in `value` (not children), so the
  // plugin can't reach it. Guard against a future regression where someone
  // turns `inlineCode` into a children-bearing node.
  const tree = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "inlineCode", value: "buzz://message?channel=c&id=m" },
        ],
      },
    ],
  };
  runPlugin(tree);
  const kids = tree.children[0].children;
  assert.equal(kids.length, 1);
  assert.equal(kids[0].type, "inlineCode");
  assert.equal(kids[0].value, "buzz://message?channel=c&id=m");
});

// ── selectProseOrNudge render-level guard ─────────────────────────────────────
//
// `MarkdownInner` calls `selectProseOrNudge(configNudge, markdownNode)` —
// the single production copy of the prose-suppression branch, exported from
// `computeConfigNudge.ts`. These tests import and call that exact function
// through a minimal stub so a revert that changes its behavior is caught
// at unit-test time.
//
// (The full `Markdown` component cannot be rendered in this environment:
// emoji-mart JSON imports crash the module loader before React runs.)

import {
  computeConfigNudge,
  selectProseOrNudge,
} from "../lib/computeConfigNudge.ts";
import { stripConfigNudgeSentinel } from "../lib/configNudge.ts";

const AGENT_PUBKEY =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
const HUMAN_PUBKEY =
  "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

function nudgeBody(agentPubkey) {
  return [
    "**Fizz** needs configuration before it can respond:",
    "- set `ANTHROPIC_API_KEY` in Edit Agent → Environment variables",
    "",
    "Open Edit Agent in the Buzz app to set these.",
    "",
    "```buzz:config-nudge",
    JSON.stringify({
      agent_name: "Fizz",
      agent_pubkey: agentPubkey,
      requirements: [{ surface: "env_key", key: "ANTHROPIC_API_KEY" }],
    }),
    "```",
  ].join("\n");
}

// Minimal wrapper that calls the real production functions from
// `computeConfigNudge.ts` — `computeConfigNudge` to detect the payload and
// `selectProseOrNudge` for the prose-suppression branch — without importing
// any Tauri or context dependencies.
function GuardStub({ content, configNudgeAuthorPubkey }) {
  const configNudge = computeConfigNudge(
    content,
    true,
    configNudgeAuthorPubkey,
  );
  const stripped =
    configNudge !== null ? stripConfigNudgeSentinel(content) : content;
  const markdownNode = React.createElement(
    "div",
    { "data-markdown-prose": "" },
    stripped,
  );
  return React.createElement(
    "div",
    null,
    selectProseOrNudge(configNudge, markdownNode),
    configNudge !== null
      ? React.createElement("div", { "data-config-nudge": "" })
      : null,
  );
}

test("nudgeGuard_sentinelPresentMatchingAuthor_cardRenderedProseAbsent", () => {
  const body = nudgeBody(AGENT_PUBKEY);
  const html = renderToStaticMarkup(
    React.createElement(GuardStub, {
      content: body,
      configNudgeAuthorPubkey: AGENT_PUBKEY,
    }),
  );
  // Card placeholder rendered.
  assert.ok(
    html.includes("data-config-nudge"),
    "data-config-nudge must be present when sentinel+author match",
  );
  // Prose suppressed — the raw fallback text must NOT appear outside the card.
  assert.ok(
    !html.includes("needs configuration before it can respond"),
    "prose must be absent when card renders",
  );
  assert.ok(
    !html.includes("data-markdown-prose"),
    "markdownNode must be null (not rendered) when configNudge is non-null",
  );
});

test("nudgeGuard_sentinelPresentWrongAuthor_proseRenderedCardAbsent", () => {
  // Sentinel present, but author pubkey is human — auth guard rejects, prose shown.
  const body = nudgeBody(AGENT_PUBKEY);
  const html = renderToStaticMarkup(
    React.createElement(GuardStub, {
      content: body,
      configNudgeAuthorPubkey: HUMAN_PUBKEY,
    }),
  );
  assert.ok(
    !html.includes("data-config-nudge"),
    "card must be absent when author pubkey does not match sentinel agent_pubkey",
  );
  assert.ok(
    html.includes("data-markdown-prose"),
    "markdownNode must render when configNudge is null (auth mismatch)",
  );
});

test("nudgeGuard_noSentinel_proseRenderedCardAbsent", () => {
  const plain = "Hello, this is a normal message with no sentinel.";
  const html = renderToStaticMarkup(
    React.createElement(GuardStub, {
      content: plain,
      configNudgeAuthorPubkey: AGENT_PUBKEY,
    }),
  );
  assert.ok(
    !html.includes("data-config-nudge"),
    "card must be absent when no sentinel is present",
  );
  assert.ok(
    html.includes("data-markdown-prose"),
    "markdownNode must render when no sentinel is present",
  );
});
