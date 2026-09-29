import assert from "node:assert/strict";
import test from "node:test";

import {
  getHuddleDockChatOpen,
  getHuddleShareBlocksAutoRead,
  getHuddleShareExpanded,
  setHuddleDockChatOpen,
  setHuddleShareExpanded,
  subscribeHuddleShareUiState,
} from "./huddleShareExpandStore.ts";

test("share expand blocks auto-read until dock chat opens or share collapses", () => {
  setHuddleShareExpanded(false);
  setHuddleDockChatOpen(false);
  assert.equal(getHuddleShareExpanded(), false);
  assert.equal(getHuddleShareBlocksAutoRead(), false);

  setHuddleShareExpanded(true);
  assert.equal(getHuddleShareBlocksAutoRead(), true);

  setHuddleDockChatOpen(true);
  assert.equal(getHuddleDockChatOpen(), true);
  assert.equal(getHuddleShareBlocksAutoRead(), false);

  setHuddleDockChatOpen(false);
  assert.equal(getHuddleShareBlocksAutoRead(), true);

  setHuddleShareExpanded(false);
  assert.equal(getHuddleDockChatOpen(), false);
  assert.equal(getHuddleShareBlocksAutoRead(), false);
});

test("subscribers fire on share / dock-chat changes", () => {
  setHuddleShareExpanded(false);
  setHuddleDockChatOpen(false);
  let ticks = 0;
  const stop = subscribeHuddleShareUiState(() => {
    ticks += 1;
  });
  setHuddleShareExpanded(true);
  setHuddleDockChatOpen(true);
  setHuddleDockChatOpen(true); // no-op
  setHuddleShareExpanded(false);
  stop();
  assert.equal(ticks, 3);
});
