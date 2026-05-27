import assert from "node:assert/strict";
import test from "node:test";

// ── Inlined pure functions from markdownUtils.ts ──────────────────────
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

function fakeElement(type) {
  return { $$typeof: REACT_ELEMENT_TYPE, type, props: {}, key: null };
}

function classifyChildren(childArray) {
  const imageChildren = childArray.filter(
    (child) => isValidElement(child) && typeof child.type !== "string",
  );
  const nonImageChildren = childArray.filter(
    (child) =>
      !(isValidElement(child) && typeof child.type !== "string") &&
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

// ── Inlined rehypeImageGallery (HAST-level) ───────────────────────────

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

// ── shallowArrayEqual ─────────────────────────────────────────────────

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

// ── classifyChildren ──────────────────────────────────────────────────

test("classifyChildren: React component elements are image children", () => {
  const ImgComponent = () => null;
  const children = [fakeElement(ImgComponent)];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 1);
  assert.equal(nonImageChildren.length, 0);
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

test("classifyChildren: mixed images, text, and br", () => {
  const Img = () => null;
  const children = [
    fakeElement(Img),
    "some text",
    fakeElement("br"),
    fakeElement(Img),
  ];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 2);
  assert.equal(nonImageChildren.length, 1); // "some text"
});

test("classifyChildren: images with only whitespace and br between them", () => {
  const Img = () => null;
  const children = [
    fakeElement(Img),
    "  ",
    fakeElement("br"),
    fakeElement(Img),
  ];
  const { imageChildren, nonImageChildren } = classifyChildren(children);
  assert.equal(imageChildren.length, 2);
  assert.equal(nonImageChildren.length, 0);
});

// ── isImageOnlyParagraph ──────────────────────────────────────────────

test("isImageOnlyParagraph: two images with br returns true", () => {
  const Img = () => null;
  const children = [fakeElement(Img), fakeElement("br"), fakeElement(Img)];
  assert.equal(isImageOnlyParagraph(children), true);
});

test("isImageOnlyParagraph: single image returns false (needs 2+)", () => {
  const Img = () => null;
  const children = [fakeElement(Img)];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("isImageOnlyParagraph: images with text returns false", () => {
  const Img = () => null;
  const children = [fakeElement(Img), "caption text", fakeElement(Img)];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("isImageOnlyParagraph: no children returns false", () => {
  assert.equal(isImageOnlyParagraph([]), false);
});

test("isImageOnlyParagraph: three images returns true", () => {
  const Img = () => null;
  const children = [fakeElement(Img), fakeElement(Img), fakeElement(Img)];
  assert.equal(isImageOnlyParagraph(children), true);
});

test("isImageOnlyParagraph: plain HTML img tags are non-image (string type)", () => {
  // <img> has type "img" (a string) — classified as non-image
  const children = [fakeElement("img"), fakeElement("img")];
  assert.equal(isImageOnlyParagraph(children), false);
});

test("isImageOnlyParagraph: mention span + images is not image-only", () => {
  const Img = () => null;
  const children = [fakeElement("span"), fakeElement(Img), fakeElement(Img)];
  assert.equal(isImageOnlyParagraph(children), false);
});

// ── hasBlockMedia ─────────────────────────────────────────────────────

test("hasBlockMedia: single image component returns true", () => {
  const Img = () => null;
  assert.equal(hasBlockMedia([fakeElement(Img)]), true);
});

test("hasBlockMedia: two images returns true", () => {
  const Img = () => null;
  assert.equal(hasBlockMedia([fakeElement(Img), fakeElement(Img)]), true);
});

test("hasBlockMedia: image with whitespace and br returns true", () => {
  const Img = () => null;
  assert.equal(
    hasBlockMedia([fakeElement(Img), "  ", fakeElement("br")]),
    true,
  );
});

test("hasBlockMedia: no children returns false", () => {
  assert.equal(hasBlockMedia([]), false);
});

test("hasBlockMedia: text only returns false", () => {
  assert.equal(hasBlockMedia(["hello"]), false);
});

test("hasBlockMedia: image with text returns false", () => {
  const Img = () => null;
  assert.equal(hasBlockMedia([fakeElement(Img), "caption"]), false);
});

test("hasBlockMedia: plain HTML img (string type) returns false", () => {
  assert.equal(hasBlockMedia([fakeElement("img")]), false);
});

// ── rehypeImageGallery (HAST-level grouping) ──────────────────────────

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

// ── messageLinkUrlTransform: sprout:// link preservation ──────────────
// Regression test: react-markdown's `defaultUrlTransform` strips unknown
// schemes (returns `""`) before our `a` component override can see them,
// which would break copy → paste → click for `sprout://message?…` links
// end-to-end. We pass a custom `urlTransform` that delegates to the
// default for everything except `sprout://message` hrefs.
//
// This test renders real `<ReactMarkdown>` with the production transform
// and asserts the link href survives to the rendered DOM. Mirrors the
// `markdown.tsx` source — keep in sync if either changes.

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";

function isMessageLink(href) {
  if (!href) return false;
  return href.startsWith("sprout://message?") || href === "sprout://message";
}

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

test("messageLinkUrlTransform: preserves sprout://message href", () => {
  const html = renderMarkdown(
    "Click [here](sprout://message?channel=abc&id=xyz)",
  );
  // HTML-encoded `&` in attributes is fine — the browser decodes back to `&`.
  assert.match(html, /href="sprout:\/\/message\?channel=abc&(?:amp;)?id=xyz"/);
});

test("messageLinkUrlTransform: preserves sprout://message href with thread", () => {
  const html = renderMarkdown(
    "[link](sprout://message?channel=c1&id=m1&thread=t1)",
  );
  assert.match(html, /href="sprout:\/\/message\?[^"]*thread=t1"/);
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

test("messageLinkUrlTransform: leaves non-message sprout:// schemes to default", () => {
  // `sprout://connect?relay=…` is handled by a different code path (Tauri
  // single-instance). The markdown renderer should let it pass through
  // defaultUrlTransform (which strips it) since it's not clickable in-app.
  const html = renderMarkdown(
    "[connect](sprout://connect?relay=wss://relay.example)",
  );
  assert.match(html, /href=""/);
});
