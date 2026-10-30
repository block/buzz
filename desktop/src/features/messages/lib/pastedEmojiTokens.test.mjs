import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";

import { CustomEmojiNode } from "./customEmojiNode.ts";
import { resolvePastedEmoji } from "./pastedEmojiTokens.ts";

const schema = getSchema([
  StarterKit.configure({ trailingNode: false, link: false }),
  CustomEmojiNode.configure({ resolveUrl: () => undefined }),
]);

function fakeView(text) {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, schema.text(text)),
  ]);
  const view = {
    state: EditorState.create({ doc }),
    isDestroyed: false,
    dispatch(tr) {
      this.state = this.state.apply(tr);
    },
  };
  return view;
}

function shape(view) {
  const parts = [];
  view.state.doc.firstChild.forEach((node) => {
    parts.push(node.isText ? node.text : `<${node.attrs.shortcode}>`);
  });
  return parts;
}

test("paste resolves every uncached name in one batch and swaps resolved tokens for nodes", async () => {
  const cache = new Map([["shipit", "https://x/shipit.png"]]);
  const batches = [];
  const resolver = {
    resolve: async (names) => {
      batches.push([...names]);
      cache.set("bufo_joy", "https://x/bufo.png");
      return {
        found: new Map([
          ["bufo_joy", { shortcode: "bufo_joy", url: "https://x/bufo.png" }],
        ]),
        failed: false,
      };
    },
  };
  const text = "a :shipit: :bufo_joy: :ghost: not:bufo_joy: :bufo_joy:";
  const view = fakeView(text);
  await resolvePastedEmoji(view, text, resolver, (name) => cache.get(name));

  assert.deepEqual(batches, [["bufo_joy", "ghost"]]);
  assert.deepEqual(shape(view), [
    "a ",
    "<shipit>",
    " ",
    "<bufo_joy>",
    " :ghost: not:bufo_joy: ",
    "<bufo_joy>",
  ]);
});

test("a failed lookup, a destroyed view, or no new names leaves the text alone", async () => {
  const text = "x :ghost: y";
  const failing = fakeView(text);
  await resolvePastedEmoji(
    failing,
    text,
    { resolve: async () => ({ found: new Map(), failed: true }) },
    () => undefined,
  );
  assert.deepEqual(shape(failing), [text]);

  const destroyed = fakeView(text);
  const cache = new Map();
  await resolvePastedEmoji(
    destroyed,
    text,
    {
      resolve: async () => {
        destroyed.isDestroyed = true;
        cache.set("ghost", "https://x/g.png");
        return { found: new Map(), failed: false };
      },
    },
    (name) => cache.get(name),
  );
  assert.deepEqual(shape(destroyed), [text]);

  let called = false;
  await resolvePastedEmoji(
    fakeView("plain"),
    "plain",
    {
      resolve: async () => {
        called = true;
        return { found: new Map(), failed: false };
      },
    },
    () => undefined,
  );
  assert.equal(called, false);
});
