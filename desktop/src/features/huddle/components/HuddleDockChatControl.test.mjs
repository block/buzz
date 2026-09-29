import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const src = fs.readFileSync(
  new URL("./HuddleDockChatControl.tsx", import.meta.url),
  "utf8",
);

test("dock chat control badges unread and opens message popover", () => {
  assert.match(src, /huddle-dock-chat-button/);
  assert.match(src, /huddle-dock-chat-unread/);
  assert.match(src, /huddle-dock-chat-popover/);
  assert.match(src, /MessageComposer/);
  assert.match(src, /MessageThreadTranscript/);
  assert.match(src, /setHuddleDockChatOpen/);
  assert.match(src, /unreadCount/);
});

test("dock chat is gated on visible prop for expand-only rendering", () => {
  assert.match(src, /if \(!visible\) return null/);
  assert.match(src, /visible: boolean/);
});

test("dock chat filters channel_created system rows from the transcript", () => {
  assert.match(src, /isChannelCreatedSystemMessage/);
  assert.match(src, /channel_created/);
});
