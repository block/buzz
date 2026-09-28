import assert from "node:assert/strict";
import test from "node:test";

import {
  beginBestieThreadSummarize,
  completeBestieThreadSummarize,
  emptyBestieThreadState,
  upsertBestieTrackedThread,
} from "./bestieThreadStorage.ts";
import { bestieThreadId } from "./bestieThreadProtocol.ts";

test("upsert tracks thread and summarize stores output", () => {
  let state = emptyBestieThreadState();
  state = upsertBestieTrackedThread(
    state,
    {
      authorName: "Ada",
      channelId: "ch1",
      channelName: "eng",
      preview: "Should we ship Friday?",
      rootEventId: "root1",
      source: "ask",
    },
    1000,
  );
  assert.equal(state.threads.length, 1);
  assert.equal(state.threads[0].id, bestieThreadId("ch1", "root1"));
  assert.equal(state.threads[0].source, "ask");

  const begun = beginBestieThreadSummarize(state, state.threads[0].id, 1100);
  assert.ok(begun);
  state = completeBestieThreadSummarize(
    begun,
    "About: ship date. Decisions: Friday. Open: docs.",
    1200,
  );
  assert.equal(state.pendingSummarize, null);
  assert.match(state.threads[0].lastSummary ?? "", /Friday/);
  assert.equal(state.threads[0].lastSummaryAt, 1200);
});
