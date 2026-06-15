import assert from "node:assert/strict";
import test from "node:test";

import remarkSpoilers from "./remarkSpoilers.ts";

function runPlugin(tree) {
  remarkSpoilers()(tree);
  return tree;
}

test("remarkSpoilers: wraps text between double-pipe delimiters", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "text", value: "keep ||secret|| visible" }],
      },
    ],
  });

  const children = tree.children[0].children;
  assert.deepEqual(children[0], { type: "text", value: "keep " });
  assert.equal(children[1].type, "spoiler");
  assert.deepEqual(children[1].data, { hName: "spoiler" });
  assert.deepEqual(children[1].children, [{ type: "text", value: "secret" }]);
  assert.deepEqual(children[2], { type: "text", value: " visible" });
});

test("remarkSpoilers: groups formatted inline nodes between delimiters", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "text", value: "||" },
          {
            type: "strong",
            children: [{ type: "text", value: "secret" }],
          },
          { type: "text", value: "||" },
        ],
      },
    ],
  });

  const children = tree.children[0].children;
  assert.equal(children.length, 1);
  assert.equal(children[0].type, "spoiler");
  assert.equal(children[0].children[0].type, "strong");
});

test("remarkSpoilers: groups inline images between delimiters", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "text", value: "||" },
          { type: "image", url: "https://example.com/secret.png", alt: "" },
          { type: "text", value: "||" },
        ],
      },
    ],
  });

  const children = tree.children[0].children;
  assert.equal(children.length, 1);
  assert.equal(children[0].type, "spoiler");
  assert.equal(children[0].children[0].type, "image");
});

test("remarkSpoilers: parses spoiler delimiters inside link labels", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          {
            type: "link",
            url: "https://example.com/a||b",
            children: [{ type: "text", value: "||secret||" }],
          },
        ],
      },
    ],
  });

  const link = tree.children[0].children[0];
  assert.equal(link.type, "link");
  assert.equal(link.url, "https://example.com/a||b");
  assert.equal(link.children.length, 1);
  assert.equal(link.children[0].type, "spoiler");
  assert.deepEqual(link.children[0].children, [
    { type: "text", value: "secret" },
  ]);
});

test("remarkSpoilers: groups block nodes between delimiter paragraphs", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "text", value: "before" }],
      },
      {
        type: "paragraph",
        children: [{ type: "text", value: "||" }],
      },
      {
        type: "paragraph",
        children: [
          { type: "image", url: "https://example.com/secret.png", alt: "" },
        ],
      },
      {
        type: "paragraph",
        children: [{ type: "text", value: "||" }],
      },
      {
        type: "paragraph",
        children: [{ type: "text", value: "after" }],
      },
    ],
  });

  assert.equal(tree.children.length, 3);
  assert.equal(tree.children[1].type, "spoiler");
  assert.deepEqual(tree.children[1].data, {
    hName: "spoiler",
    hProperties: { "data-block-spoiler": "" },
  });
  assert.equal(tree.children[1].children[0].children[0].type, "image");
});

test("remarkSpoilers: leaves unmatched delimiters as text", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "text", value: "keep ||plain" }],
      },
    ],
  });

  assert.deepEqual(tree.children[0].children, [
    { type: "text", value: "keep " },
    { type: "text", value: "||" },
    { type: "text", value: "plain" },
  ]);
});

test("remarkSpoilers: does not parse delimiters inside inline code", () => {
  const tree = runPlugin({
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "inlineCode", value: "||secret||" }],
      },
    ],
  });

  assert.deepEqual(tree.children[0].children, [
    { type: "inlineCode", value: "||secret||" },
  ]);
});
