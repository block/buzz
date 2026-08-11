import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

import {
  registerComposerMessageLinkMarkdownIt,
  resolveComposerMessageLinkAttributes,
} from "./composerMessageLinkNode.ts";

const requireFromTiptap = createRequire(import.meta.resolve("tiptap-markdown"));
const MarkdownIt = requireFromTiptap("markdown-it");

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const MESSAGE_ID = "root-event";
const HREF = `buzz://message?channel=${CHANNEL_ID}&id=${MESSAGE_ID}`;

test("resolves a composer preview and canonicalizes the underlying href", () => {
  assert.deepEqual(
    resolveComposerMessageLinkAttributes(
      HREF.replace("buzz://", "BUZZ://"),
      (channelId) => (channelId === CHANNEL_ID ? "general" : undefined),
    ),
    { channelName: "general", href: HREF },
  );
});

test("rejects malformed message links", () => {
  assert.equal(
    resolveComposerMessageLinkAttributes(
      `buzz://message?channel=${CHANNEL_ID}`,
      () => "general",
    ),
    null,
  );
});

function captureMarkdownRule() {
  let capturedAnchor = null;
  let capturedRule = null;
  const md = {
    renderer: { rules: {} },
    inline: {
      ruler: {
        before(anchor, _name, rule) {
          capturedAnchor = anchor;
          capturedRule = rule;
        },
      },
    },
    utils: {
      escapeHtml: (value) => value.replaceAll("&", "&amp;"),
    },
  };
  registerComposerMessageLinkMarkdownIt(md, {
    resolveChannelName: (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
  });
  return { anchor: capturedAnchor, md, rule: capturedRule };
}

test("markdown parsing materializes a bare message link in composer content", () => {
  const { anchor, rule } = captureMarkdownRule();
  assert.equal(anchor, "text");
  let token = null;
  const state = {
    src: `See ${HREF}.`,
    pos: 4,
    push: () => {
      token = { meta: null };
      return token;
    },
  };

  assert.equal(rule(state, false), true);
  assert.equal(state.pos, 4 + HREF.length);
  assert.deepEqual(token.meta, { channelName: "general", href: HREF });
});

test("real markdown-it parsing materializes a restored message link", () => {
  const md = new MarkdownIt();
  registerComposerMessageLinkMarkdownIt(md, {
    resolveChannelName: (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
  });

  const html = md.renderInline(`See ${HREF}.`);
  assert.match(html, /See <span data-composer-message-link=""/);
  assert.match(html, /data-channel-name="general"/);
  assert.match(html, /data-href="buzz:\/\/message\?channel=.*&amp;id=/);
});

test("markdown parsing resumes after markdown-it consumes the buzz prefix", () => {
  const { rule } = captureMarkdownRule();
  let token = null;
  const state = {
    pending: "See buzz",
    src: `See ${HREF}`,
    pos: "See buzz".length,
    push: () => {
      token = { meta: null };
      return token;
    },
  };

  assert.equal(rule(state, false), true);
  assert.equal(state.pending, "See ");
  assert.equal(state.pos, state.src.length);
  assert.deepEqual(token.meta, { channelName: "general", href: HREF });
});

test("markdown parsing stops message links before emphasis delimiters", () => {
  const { rule } = captureMarkdownRule();
  let token = null;
  const state = {
    src: `${HREF}*`,
    pos: 0,
    push: () => {
      token = { meta: null };
      return token;
    },
  };

  assert.equal(rule(state, false), true);
  assert.equal(state.pos, HREF.length);
  assert.deepEqual(token.meta, { channelName: "general", href: HREF });
});

test("markdown rendering stores identity in attributes, not visible id text", () => {
  const { md } = captureMarkdownRule();
  const render = md.renderer.rules.buzz_composer_message_link;
  const html = render([{ meta: { channelName: "general", href: HREF } }], 0);

  assert.match(html, /data-composer-message-link=""/);
  assert.match(html, /data-channel-name="general"/);
  assert.match(html, /data-href="buzz:\/\/message\?channel=.*&amp;id=/);
  assert.doesNotMatch(html, />[^<]*root-event/);
});
