import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  clearComposerDrafts,
  consumeComposerDraft,
  FRESH_DRAFT_KEY,
  loadComposerDraft,
  saveComposerDraft,
  saveComposerDraftIfEmpty,
} from "./composerDrafts.ts";

beforeEach(() => {
  clearComposerDrafts();
});

test("drafts round-trip per key and never bleed across keys", () => {
  saveComposerDraft("channel-a", "draft for a");
  saveComposerDraft("channel-b", "draft for b");

  assert.equal(loadComposerDraft("channel-a"), "draft for a");
  assert.equal(loadComposerDraft("channel-b"), "draft for b");
  assert.equal(loadComposerDraft("channel-c"), "");
});

test("the fresh composer has its own slot", () => {
  saveComposerDraft(FRESH_DRAFT_KEY, "spawn a new channel");
  saveComposerDraft("channel-a", "channel text");

  assert.equal(loadComposerDraft(FRESH_DRAFT_KEY), "spawn a new channel");
  assert.equal(loadComposerDraft("channel-a"), "channel text");
});

test("whitespace-only text clears the slot", () => {
  saveComposerDraft("channel-a", "kept");
  saveComposerDraft("channel-a", "  \n\t ");

  assert.equal(loadComposerDraft("channel-a"), "");
});

test("consuming a draft empties the slot", () => {
  saveComposerDraft("channel-a", "sent text");
  consumeComposerDraft("channel-a");

  assert.equal(loadComposerDraft("channel-a"), "");
});

test("saveComposerDraftIfEmpty never overwrites an existing draft", () => {
  saveComposerDraftIfEmpty("channel-a", "failed prompt");
  assert.equal(loadComposerDraft("channel-a"), "failed prompt");

  saveComposerDraftIfEmpty("channel-a", "second failure");
  assert.equal(loadComposerDraft("channel-a"), "failed prompt");
});
