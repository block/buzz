import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

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
      !(
        isValidElement(child) &&
        (child.type === "br" || child.props?.node?.tagName === "br")
      ),
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
    (child) => !isIgnorableImageSeparator(child),
  );
  return (
    meaningful.length >= 1 &&
    meaningful.every((child) => isHastElement(child) && child.tagName === "img")
  );
}

function isIgnorableImageSeparator(node) {
  return (
    (isHastText(node) && node.value.trim() === "") ||
    (isHastElement(node) && node.tagName === "br")
  );
}

function splitTrailingImageRun(node) {
  if (!isHastElement(node) || node.tagName !== "p") return [node];

  let cursor = node.children.length - 1;
  const trailingImages = [];
  while (cursor >= 0) {
    const child = node.children[cursor];
    if (isHastElement(child) && child.tagName === "img") {
      trailingImages.unshift(child);
      cursor -= 1;
      continue;
    }
    if (isIgnorableImageSeparator(child)) {
      cursor -= 1;
      continue;
    }
    break;
  }

  if (trailingImages.length < 2 || cursor < 0) return [node];
  return [
    { ...node, children: node.children.slice(0, cursor + 1) },
    {
      type: "element",
      tagName: "p",
      properties: {},
      children: trailingImages,
    },
  ];
}

function rehypeImageGallery() {
  return (tree) => {
    const normalizedChildren = tree.children.flatMap(splitTrailingImageRun);
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

    for (const child of normalizedChildren) {
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

test("classifyChildren: react-markdown break components are excluded", () => {
  const BreakComponent = () => null;
  const children = [
    fakeElement(BreakComponent, { node: { type: "element", tagName: "br" } }),
  ];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 0);
  assert.equal(nonImageChildren.length, 0);
});

test("isImageOnlyParagraph: react-markdown breaks preserve image mosaics", () => {
  const BreakComponent = () => null;
  const media = { "data-block-media": "" };
  const customBreak = fakeElement(BreakComponent, {
    node: { type: "element", tagName: "br" },
  });
  const children = [
    fakeElement("span", media),
    customBreak,
    fakeElement("span", media),
    customBreak,
    fakeElement("span", media),
  ];
  assert.equal(isImageOnlyParagraph(children), true);
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

test("rehypeImageGallery: splits composer text from trailing image bundle", () => {
  const br = { type: "element", tagName: "br", properties: {}, children: [] };
  const tree = {
    type: "root",
    children: [
      hastP(
        hastText("gallery bundle"),
        br,
        hastImg("a.png"),
        br,
        hastImg("b.png"),
        br,
        hastImg("c.png"),
      ),
    ],
  };

  rehypeImageGallery()(tree);

  assert.equal(tree.children.length, 2);
  assert.equal(tree.children[0].children[0].value, "gallery bundle");
  assert.deepEqual(
    tree.children[1].children.map((child) => child.properties.src),
    ["a.png", "b.png", "c.png"],
  );
});

test("rehypeImageGallery: leaves a single trailing image in the text flow", () => {
  const br = { type: "element", tagName: "br", properties: {}, children: [] };
  const paragraph = hastP(hastText("caption"), br, hastImg("a.png"));
  const tree = { type: "root", children: [paragraph] };

  rehypeImageGallery()(tree);

  assert.equal(tree.children.length, 1);
  assert.equal(tree.children[0], paragraph);
});

// Regression test: react-markdown's `defaultUrlTransform` strips unknown
// schemes (returns `""`) before our `a` component override can see them,
// which would break copy → paste → click for `buzz://message?…` links and
// `buzz://pr|issue|repo?…` entity links end-to-end. We pass a custom
// `urlTransform` (`buzzDeepLinkUrlTransform`) that preserves valid Buzz
// deep links and delegates everything else to `defaultUrlTransform`.
//
// This test renders real `<ReactMarkdown>` with the production transform
// and asserts the link href survives to the rendered DOM. Mirrors the
// `markdown.tsx` source — keep in sync if either changes.

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";

import { isChannelLink } from "../../features/messages/lib/channelLink.ts";
import { isMessageLink } from "../../features/messages/lib/messageLink.ts";
import { parseEntityLink } from "../lib/entityLink.ts";
import remarkSpoilers from "../lib/remarkSpoilers.ts";

const OWNER_HEX =
  "71d67180ba17e749ee825fc8819c9c6ee7003617e1c126504f9b658070ab9224";
const EVENT_HEX =
  "c3b589fa5713ba25bad6dc095e2de00a4ac8f50050fdea00fc6444e603be1dd1";

function buzzDeepLinkUrlTransform(value, key) {
  if (key !== "href") return defaultUrlTransform(value);
  if (isMessageLink(value) || isChannelLink(value)) return value;
  if (parseEntityLink(value).ok) return value;
  return defaultUrlTransform(value);
}

function renderMarkdown(content) {
  return renderToStaticMarkup(
    React.createElement(
      ReactMarkdown,
      { urlTransform: buzzDeepLinkUrlTransform },
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

test("messageLinkUrlTransform: preserves buzz://channel href", () => {
  const html = renderMarkdown(
    "Click [here](buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32)",
  );
  assert.match(
    html,
    /href="buzz:\/\/channel\/580ca78b-9dae-46f3-8854-bd671853ba32"/,
  );
});

test("messageLinkUrlTransform: rejects malformed buzz://channel href", () => {
  const html = renderMarkdown(
    "Click [here](buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32?extra=true)",
  );
  assert.match(html, /href=""/);
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

test("messageLinkUrlTransform: leaves non-entity buzz:// schemes to default", () => {
  // `buzz://connect?relay=…` is handled by a different code path (Tauri
  // single-instance). The markdown renderer should let it pass through
  // defaultUrlTransform (which strips it) since it's not clickable in-app.
  const html = renderMarkdown(
    "[connect](buzz://connect?relay=wss://relay.example)",
  );
  assert.match(html, /href=""/);
});

test("buzzDeepLinkUrlTransform: preserves buzz://pr entity link href", () => {
  const prLink = `buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`;
  const html = renderMarkdown(`[My PR](${prLink})`);
  // The href must survive — our transform preserves valid entity links.
  assert.match(html, /href="buzz:\/\/pr\?/);
  assert.doesNotMatch(html, /href=""/);
});

test("buzzDeepLinkUrlTransform: preserves buzz://pr autolink href", () => {
  const prLink = `buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`;
  const html = renderMarkdown(`<${prLink}>`);
  assert.match(html, /href="buzz:\/\/pr\?/);
  assert.doesNotMatch(html, /href=""/);
});

test("buzzDeepLinkUrlTransform: preserves buzz://issue entity link href", () => {
  const issueLink = `buzz://issue?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`;
  const html = renderMarkdown(`[Issue title](${issueLink})`);
  assert.match(html, /href="buzz:\/\/issue\?/);
  assert.doesNotMatch(html, /href=""/);
});

test("buzzDeepLinkUrlTransform: preserves buzz://repo entity link href", () => {
  const repoLink = `buzz://repo?owner=${OWNER_HEX}&d=buzz-world`;
  const html = renderMarkdown(`[My repo](${repoLink})`);
  assert.match(html, /href="buzz:\/\/repo\?/);
  assert.doesNotMatch(html, /href=""/);
});

test("buzzDeepLinkUrlTransform: preserves buzz://project autolink href", () => {
  const projectLink = `buzz://project?owner=${OWNER_HEX}&d=onboarding`;
  const html = renderMarkdown(`<${projectLink}>`);
  assert.match(html, /href="buzz:\/\/project\?/);
  assert.doesNotMatch(html, /href=""/);
});

test("buzzDeepLinkUrlTransform: strips malformed buzz://pr (unknown param)", () => {
  // Strict parser rejects unknown params — transform falls back to default sanitizer.
  const html = renderMarkdown(
    `[link](buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world&extra=ignored)`,
  );
  assert.match(html, /href=""/);
});

// ── renderEntityLinkAnchor: clone-URL anchor origin gating ───────────────────
//
// `renderEntityLinkAnchor` now accepts `relayOrigin` and passes it to
// `parseSupportedLinkPreview`. A clone URL is only treated as an in-app entity
// link when the URL origin exactly matches the active relay origin. This covers
// the inline anchor click path (not just card extraction).

import { renderEntityLinkAnchor } from "../ui/markdown/entityLinks.tsx";
import { createMarkdownComponents } from "../ui/markdown.tsx";
import { renderCachedMarkdown } from "../ui/markdown/nodeCache.ts";
import { MarkdownRuntimeContext } from "../ui/markdown/runtimeContext.ts";

const CLONE_URL = `https://relay.example/git/${OWNER_HEX}/my-repo`;

test("renderEntityLinkAnchor_matchingOriginCloneUrl_returnsEntityAnchor", () => {
  // Origin matches active relay — anchor should navigate in-app (non-null).
  const el = renderEntityLinkAnchor({
    children: React.createElement("span", null, "my-repo"),
    href: CLONE_URL,
    onOpenEntityLink: () => {},
    relayOrigin: "https://relay.example",
  });
  assert.notEqual(
    el,
    null,
    "matching-origin clone URL must produce an entity anchor",
  );
  const html = renderToStaticMarkup(el);
  // The entity anchor must be rendered — it carries the original href (for display)
  // and navigates in-app via onClick. Verify the anchor is present with the cursor style.
  assert.match(
    html,
    /cursor-pointer/,
    "entity anchor must have the in-app navigation cursor style",
  );
});

test("renderEntityLinkAnchor_lookalikeDomainCloneUrl_returnsNull", () => {
  // Origin does NOT match active relay — must fall through to ExternalLinkAnchor.
  const el = renderEntityLinkAnchor({
    children: React.createElement("span", null, "my-repo"),
    href: CLONE_URL,
    onOpenEntityLink: () => {},
    relayOrigin: "https://evil.example",
  });
  assert.equal(
    el,
    null,
    "lookalike-origin clone URL must return null so the anchor falls through to external",
  );
});

test("renderEntityLinkAnchor_noRelayOrigin_cloneUrlReturnsNull", () => {
  // No known relay origin — must fail closed, not guess.
  const el = renderEntityLinkAnchor({
    children: React.createElement("span", null, "my-repo"),
    href: CLONE_URL,
    onOpenEntityLink: () => {},
    relayOrigin: null,
  });
  assert.equal(
    el,
    null,
    "clone URL without a relay origin must return null (fail closed)",
  );
});

test("renderEntityLinkAnchor_directEntityLink_returnsAnchorRegardlessOfOrigin", () => {
  // A direct buzz://pr link always resolves in-app — it does not require origin.
  const prLink = `buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`;
  const el = renderEntityLinkAnchor({
    children: React.createElement("span", null, "My PR"),
    href: prLink,
    onOpenEntityLink: () => {},
    relayOrigin: null,
  });
  assert.notEqual(
    el,
    null,
    "direct buzz://pr link must produce an entity anchor regardless of origin",
  );
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
  selectNudgeLeadingContent,
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
function GuardStub({ content, configNudgeAuthorPubkey, leadingInlineContent }) {
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
      ? React.createElement(
          "div",
          { "data-config-nudge": "" },
          selectNudgeLeadingContent(configNudge, leadingInlineContent),
        )
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

test("nudgeGuard_sentinelPresentMatchingAuthor_preservesLeadingContent", () => {
  const body = nudgeBody(AGENT_PUBKEY);
  const html = renderToStaticMarkup(
    React.createElement(GuardStub, {
      content: body,
      configNudgeAuthorPubkey: AGENT_PUBKEY,
      leadingInlineContent: React.createElement(
        "span",
        { "data-video-review-timecode": "" },
        "[00:10]",
      ),
    }),
  );
  assert.ok(
    html.includes("data-video-review-timecode"),
    "leading video-review content must remain visible beside the nudge card",
  );
  assert.ok(
    !html.includes("data-markdown-prose"),
    "nudge prose must remain suppressed while leading content is preserved",
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

test("bare Buzz permalinks render cohesive icon-prefixed chips", () => {
  const channelId = "580ca78b-9dae-46f3-8854-bd671853ba32";
  const messageLink = `buzz://message?channel=${channelId}&id=${EVENT_HEX}`;
  const compatibilityMessageLink = `buzz://channel/${channelId}/${EVENT_HEX}`;
  const channelLink = `buzz://channel/${channelId}`;
  const links = [
    messageLink,
    compatibilityMessageLink,
    channelLink,
    `buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`,
    `buzz://issue?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`,
    `buzz://repo?owner=${OWNER_HEX}&d=buzz-world`,
  ];
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content: links.join(" "),
    variant: "entity-link-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      QueryClientProvider,
      { client: new QueryClient() },
      React.createElement(
        MarkdownRuntimeContext.Provider,
        {
          value: {
            channels: [{ id: channelId, name: "engineering" }],
            onOpenChannel: () => {},
            onOpenEntityLink: () => {},
            onOpenMessageLink: () => {},
            relayOrigin: null,
          },
        },
        markdown,
      ),
    ),
  );

  const visibleText = html.replace(/<[^>]+>/g, "");
  assert.equal((html.match(/data-buzz-link=""/g) ?? []).length, 6);
  assert.equal(
    (
      html.match(
        /inline-chip-leading-fragment[^"]*inline-chip-icon-message/g,
      ) ?? []
    ).length,
    2,
  );
  assert.equal((visibleText.match(/engineering/g) ?? []).length, 3);
  assert.equal((html.match(/data-message-link=""/g) ?? []).length, 2);
  assert.equal((html.match(/data-channel-deep-link=""/g) ?? []).length, 1);
  assert.match(html, /inline-chip-icon-channel/);
  assert.match(html, /wrapping-inline-chip/);
  assert.match(html, /inline-chip-leading-fragment[^>]*>engin</);
  assert.match(html, /inline-chip-icon-pr/);
  assert.match(html, /inline-chip-icon-issue/);
  assert.match(html, /inline-chip-icon-repo/);
  // PR, issue, and repository chips all use the stable repository identity;
  // fetched subjects and event hashes never alter their inline width.
  assert.equal((visibleText.match(/buzz-world/g) ?? []).length, 3);
  assert.doesNotMatch(visibleText, /c3b589fa/);
});

test("inline issue and pull-request chips show the repository name without the event hash", () => {
  const renderEntityChip = (href) =>
    renderToStaticMarkup(
      renderEntityLinkAnchor({
        children: null,
        href,
        onOpenEntityLink: () => {},
        relayOrigin: null,
      }),
    );

  const issueHtml = renderEntityChip(
    `buzz://issue?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`,
  );
  const issueText = issueHtml.replace(/<[^>]+>/g, "");
  assert.equal(issueText, "buzz-world");
  assert.doesNotMatch(issueText, /c3b589fa/);
  assert.doesNotMatch(issueText, /·/);
  // Identity, icon, and navigation affordances survive the shorter label.
  assert.match(issueHtml, /data-buzz-link-kind="issue"/);
  assert.match(issueHtml, /inline-chip-icon-issue/);
  assert.match(
    issueHtml,
    /aria-label="Open issue c3b589fa in repository buzz-world"/,
  );

  // Pull-request chips follow the same stable inline identity policy.
  const pullRequestText = renderEntityChip(
    `buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`,
  ).replace(/<[^>]+>/g, "");
  assert.equal(pullRequestText, "buzz-world");
  assert.doesNotMatch(pullRequestText, /c3b589fa/);
  assert.doesNotMatch(pullRequestText, /·/);
});

test("inline entity chip labels are bounded without truncating their accessible names", () => {
  const longRepository = `repo-${"x".repeat(50)}`;
  const html = renderToStaticMarkup(
    renderEntityLinkAnchor({
      children: null,
      href: `buzz://repo?owner=${OWNER_HEX}&d=${longRepository}`,
      onOpenEntityLink: () => {},
      relayOrigin: null,
    }),
  );
  const visibleText = html.replace(/<[^>]+>/g, "");
  assert.equal(Array.from(visibleText).length, 48);
  assert.match(visibleText, /…$/);
  assert.match(
    html,
    new RegExp(`aria-label="Open repository ${longRepository}"`),
  );
});

test("inline message chips omit fetched metadata and the event hash", () => {
  const channelId = "580ca78b-9dae-46f3-8854-bd671853ba32";
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content: `buzz://message?channel=${channelId}&id=${EVENT_HEX}`,
    variant: "inline-message-chip-metadata-test",
  });
  // A readable channel is the case that used to swap the chip label from the
  // truncated event hash to the fetched snippet once metadata resolved.
  const html = renderToStaticMarkup(
    React.createElement(
      QueryClientProvider,
      { client: new QueryClient() },
      React.createElement(
        MarkdownRuntimeContext.Provider,
        {
          value: {
            channels: [
              {
                id: channelId,
                isMember: true,
                name: "engineering",
                visibility: "open",
              },
            ],
            onOpenChannel: () => {},
            onOpenEntityLink: () => {},
            onOpenMessageLink: () => {},
            relayOrigin: null,
          },
        },
        markdown,
      ),
    ),
  );

  const visibleText = html.replace(/<[^>]+>/g, "");
  assert.equal((html.match(/data-message-link=""/g) ?? []).length, 1);
  assert.equal(visibleText.trim(), "engineering");
  assert.doesNotMatch(visibleText, /c3b589fa/);
  assert.doesNotMatch(visibleText, /·/);
});

test("authored Buzz permalink labels remain ordinary links", () => {
  const channelId = "580ca78b-9dae-46f3-8854-bd671853ba32";
  const links = [
    `[the message](buzz://message?channel=${channelId}&id=${EVENT_HEX})`,
    `[the compatibility message](buzz://channel/${channelId}/${EVENT_HEX})`,
    `[**design discussion**](buzz://channel/${channelId})`,
    `[the issue](buzz://issue?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world)`,
  ];
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content: links.join(" "),
    variant: "authored-buzz-link-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      QueryClientProvider,
      { client: new QueryClient() },
      React.createElement(
        MarkdownRuntimeContext.Provider,
        {
          value: {
            channels: [{ id: channelId, name: "engineering" }],
            onOpenChannel: () => {},
            onOpenEntityLink: () => {},
            onOpenMessageLink: () => {},
            relayOrigin: null,
          },
        },
        markdown,
      ),
    ),
  );

  assert.equal((html.match(/data-buzz-link=""/g) ?? []).length, 0);
  assert.match(html, />the message</);
  assert.match(html, />the compatibility message</);
  assert.match(html, /aria-label="Open message: the compatibility message"/);
  assert.match(html, />design discussion</);
  assert.match(html, /aria-label="Open channel: design discussion"/);
  assert.doesNotMatch(html, /\[object Object\]/);
  assert.match(html, />the issue</);
  assert.equal((html.match(/underline-offset-4/g) ?? []).length, 4);
});

test("generic audio attachments render outside paragraph markup", () => {
  const href = "https://relay.example/media/meeting.mp3";
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content: `[meeting.mp3](${href})`,
    variant: "generic-audio-block-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      MarkdownRuntimeContext.Provider,
      {
        value: {
          channels: [],
          imetaByUrl: new Map([
            [
              href,
              {
                duration: 42,
                filename: "meeting.mp3",
                m: "audio/mpeg",
              },
            ],
          ]),
          onOpenChannel: () => {},
          onOpenEntityLink: () => {},
          onOpenMessageLink: () => {},
          relayOrigin: "https://relay.example",
        },
      },
      markdown,
    ),
  );

  assert.match(html, /data-testid="audio-message-attachment"/);
  assert.match(html, /aria-label="Download meeting.mp3"/);
  assert.doesNotMatch(html, /<p[^>]*>\s*<div/);
});

test("bare Buzz permalinks shorten unavailable channel identifiers", () => {
  const channelId = "580ca78b-9dae-46f3-8854-bd671853ba32";
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content: [
      `buzz://message?channel=${channelId}&id=${EVENT_HEX}`,
      `buzz://channel/${channelId}`,
    ].join(" "),
    variant: "unknown-channel-buzz-link-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      MarkdownRuntimeContext.Provider,
      {
        value: {
          channels: [],
          onOpenChannel: () => {},
          onOpenEntityLink: () => {},
          onOpenMessageLink: () => {},
          relayOrigin: null,
        },
      },
      markdown,
    ),
  );

  assert.equal(
    (html.match(/inline-chip-leading-fragment[^>]*>580ca<\/span>78b/g) ?? [])
      .length,
    2,
  );
  assert.doesNotMatch(html, /#channel/);
});

test("channel references replace the authored hash with the channel icon", () => {
  const channelId = "580ca78b-9dae-46f3-8854-bd671853ba32";
  const markdown = renderCachedMarkdown({
    channelNames: ["engineering"],
    components: createMarkdownComponents(true, false),
    content: "See #engineering",
    variant: "channel-reference-icon-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      MarkdownRuntimeContext.Provider,
      {
        value: {
          channels: [{ id: channelId, name: "engineering" }],
          onOpenChannel: () => {},
          onOpenEntityLink: () => {},
          onOpenMessageLink: () => {},
          relayOrigin: null,
        },
      },
      markdown,
    ),
  );

  assert.match(html, /inline-chip-icon-channel/);
  assert.match(html, /wrapping-inline-chip/);
  assert.match(html, /inline-chip-leading-fragment[^>]*>engin</);
  assert.match(html.replace(/<[^>]+>/g, ""), /engineering/);
  assert.doesNotMatch(html, />#engineering</);
});

test("resolved human mentions replace the authored at-sign with the shared icon", () => {
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(false, false),
    content: "Ask @alice",
    mentionNames: ["alice"],
    variant: "human-mention-icon-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      MarkdownRuntimeContext.Provider,
      {
        value: {
          channels: [],
          mentionPubkeysByName: { alice: HUMAN_PUBKEY },
          onOpenChannel: () => {},
          onOpenEntityLink: () => {},
          onOpenMessageLink: () => {},
          relayOrigin: null,
        },
      },
      markdown,
    ),
  );

  assert.match(html, /data-mention=""/);
  assert.match(html, /inline-chip-icon-human/);
  assert.match(html, />alice</);
  assert.doesNotMatch(html, />@alice</);
});

test("agent mentions retain the bot treatment instead of the human icon", () => {
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(false, false),
    content: "Ask @alice",
    mentionNames: ["alice"],
    variant: "agent-mention-icon-integration-test",
  });
  const html = renderToStaticMarkup(
    React.createElement(
      MarkdownRuntimeContext.Provider,
      {
        value: {
          agentMentionPubkeysByName: { alice: AGENT_PUBKEY },
          channels: [],
          mentionPubkeysByName: { alice: AGENT_PUBKEY },
          onOpenChannel: () => {},
          onOpenEntityLink: () => {},
          onOpenMessageLink: () => {},
          relayOrigin: null,
        },
      },
      markdown,
    ),
  );

  assert.match(html, /data-mention=""/);
  assert.match(html, /agent-mention-highlight/);
  assert.match(html, /inline-chip-icon-agent/);
  assert.match(html, />alice</);
  assert.doesNotMatch(html, />@alice</);
});

test("renderEntityLinkAnchor renders Buzz entity links as chips", () => {
  const prLink = `buzz://pr?id=${EVENT_HEX}&owner=${OWNER_HEX}&d=buzz-world`;
  const el = renderEntityLinkAnchor({
    children: "PR · abc123",
    href: prLink,
    interactive: true,
    onOpenEntityLink: () => {},
    relayOrigin: null,
  });
  const html = renderToStaticMarkup(el);
  assert.match(html, /data-buzz-link=""/);
  assert.match(html, /<span/);
  assert.match(html, /role="button"/);
  assert.match(html, /tabindex="0"/);
  assert.match(html, /data-buzz-link-kind="pr"/);
  assert.match(html, /wrapping-inline-chip/);
  assert.match(html, /inline-chip-leading-fragment[^>]*>buzz-</);
  assert.doesNotMatch(html, /\btruncate\b/);
  assert.doesNotMatch(html, /<a/);
  assert.doesNotMatch(html, /<button/);
});

test("renderEntityLinkAnchor keeps chip styling when interaction is disabled", () => {
  const repoLink = `buzz://repo?owner=${OWNER_HEX}&d=buzz-world`;
  const el = renderEntityLinkAnchor({
    children: "buzz-world",
    href: repoLink,
    interactive: false,
    onOpenEntityLink: () => {},
    relayOrigin: null,
  });
  const html = renderToStaticMarkup(el);
  assert.match(html, /data-buzz-link=""/);
  assert.match(html, /<span/);
  assert.match(html, /class="mention-chip\s/);
  assert.doesNotMatch(html, /<button/);
});
