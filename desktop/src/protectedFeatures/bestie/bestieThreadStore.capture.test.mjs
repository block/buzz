import assert from "node:assert/strict";
import test from "node:test";

import {
  __resetBestieThreadStoreForTests,
  abandonBestieThreadSummarizeForScope,
  applyBestieThreadSummarizeReply,
  beginBestieThreadSummarizeForScope,
  getBestieThreadState,
  setBestieThreadSummarizeTriggerForScope,
  upsertBestieTrackedThreadForScope,
} from "./bestieThreadStore.ts";
import { BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT } from "./bestieThreadSummarizeLive.ts";

const SCOPE = {
  agentPubkey: "c".repeat(64),
  ownerPubkey: "d".repeat(64),
  relayUrl: "wss://relay.test",
};

function memoryWindow() {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };
}

test("stores failure summarize reply on the thread row", () => {
  memoryWindow();
  __resetBestieThreadStoreForTests();
  upsertBestieTrackedThreadForScope(SCOPE, {
    channelId: "chan-1",
    preview: "hello",
    rootEventId: "root-1",
    source: "add",
  });
  const threadId = "chan-1:root-1";
  const startedAt = 1_700_000_000;
  // begin uses Date.now by default — patch via storage path: begin then set times by completing carefully
  beginBestieThreadSummarizeForScope(SCOPE, threadId);
  // Force startedAt by writing pending through trigger after begin
  const state = getBestieThreadState(SCOPE);
  assert.ok(state.pendingSummarize);
  setBestieThreadSummarizeTriggerForScope(SCOPE, "sum-trigger-1");

  const ok = applyBestieThreadSummarizeReply(
    SCOPE,
    "agent-fail-1",
    "Could not read thread: restricted: not a channel member",
    Math.floor(Date.now() / 1000) + 10,
    [
      ["e", "sum-trigger-1", "", "root"],
      ["e", "sum-trigger-1", "", "reply"],
    ],
  );
  assert.equal(ok, true);
  const thread = getBestieThreadState(SCOPE).threads[0];
  assert.match(thread.lastSummary ?? "", /restricted/);
  assert.equal(getBestieThreadState(SCOPE).pendingSummarize, null);
});

test("does not capture coffee reply while summarize pending", () => {
  memoryWindow();
  __resetBestieThreadStoreForTests();
  upsertBestieTrackedThreadForScope(SCOPE, {
    channelId: "chan-2",
    preview: "hello",
    rootEventId: "root-2",
    source: "ask",
  });
  beginBestieThreadSummarizeForScope(SCOPE, "chan-2:root-2");
  setBestieThreadSummarizeTriggerForScope(SCOPE, "sum-trigger-2");
  const ok = applyBestieThreadSummarizeReply(
    SCOPE,
    "agent-coffee-1",
    "NCPs aren't enabled on this agent.",
    Math.floor(Date.now() / 1000) + 10,
    [
      ["e", "coffee-trigger", "", "root"],
      ["e", "coffee-trigger", "", "reply"],
    ],
  );
  assert.equal(ok, false);
  assert.ok(getBestieThreadState(SCOPE).pendingSummarize);
  assert.equal(getBestieThreadState(SCOPE).threads[0].lastSummary, null);
});

test("abandon summarize with trigger writes failure on the row", () => {
  memoryWindow();
  __resetBestieThreadStoreForTests();
  upsertBestieTrackedThreadForScope(SCOPE, {
    channelId: "chan-abandon",
    preview: "hello",
    rootEventId: "root-abandon",
    source: "add",
  });
  beginBestieThreadSummarizeForScope(SCOPE, "chan-abandon:root-abandon");
  setBestieThreadSummarizeTriggerForScope(SCOPE, "sum-trigger-abandon");
  abandonBestieThreadSummarizeForScope(SCOPE);
  const state = getBestieThreadState(SCOPE);
  assert.equal(state.pendingSummarize, null);
  assert.equal(
    state.threads[0].lastSummary,
    BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT,
  );
});
