import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_THREAD_SUMMARIZE_HARD_TIMEOUT_SECONDS,
  BESTIE_THREAD_SUMMARIZE_IDLE_SETTLE_SECONDS,
  BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS,
  isBestieThreadSummarizeLive,
  isBestieThreadSummarizePendingStale,
  messageLooksLikeBestieSummarizeCompetingTrigger,
  messageLooksLikeBestieSummarizeTrigger,
  shouldDisableBestieThreadSummarize,
} from "./bestieThreadSummarizeLive.ts";
import { BESTIE_THREAD_SUMMARIZE_MARKER } from "./bestieThreadProtocol.ts";

test("summarize trigger and competing (coffee/job/reminder) detection", () => {
  assert.equal(
    messageLooksLikeBestieSummarizeTrigger(
      `${BESTIE_THREAD_SUMMARIZE_MARKER}\n\nSummarize…`,
    ),
    true,
  );
  assert.equal(
    messageLooksLikeBestieSummarizeCompetingTrigger(
      "[Bestie coffee]\n\n/hula-coffee",
    ),
    true,
  );
  assert.equal(
    messageLooksLikeBestieSummarizeCompetingTrigger(
      `${BESTIE_THREAD_SUMMARIZE_MARKER}\n\nok`,
    ),
    false,
  );
});

test("not live when Coffee is newer than summarize pending", () => {
  assert.equal(
    isBestieThreadSummarizeLive({
      agentWorkingOnBestieDm: true,
      nowSeconds: 5_000,
      latestCompetingTriggerAt: 4_000,
      pendingSummarize: {
        startedAt: 1_000,
        threadId: "ch:root",
        triggerMessageId: "sum-1",
      },
    }),
    false,
  );
  assert.equal(
    shouldDisableBestieThreadSummarize({
      agentWorkingOnBestieDm: true,
      nowSeconds: 5_000,
      latestCompetingTriggerAt: 4_000,
      pendingSummarize: {
        startedAt: 1_000,
        threadId: "ch:root",
        triggerMessageId: "sum-1",
      },
    }),
    false,
  );
});

test("live during start grace when summarize is latest", () => {
  const startedAt = 1_000;
  assert.equal(
    isBestieThreadSummarizeLive({
      agentWorkingOnBestieDm: false,
      nowSeconds: startedAt + BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS,
      pendingSummarize: {
        startedAt,
        threadId: "ch:root",
        triggerMessageId: null,
      },
    }),
    true,
  );
});

test("stale summarize pending after idle settle or hard timeout", () => {
  const startedAt = 2_000;
  const settleAt =
    startedAt +
    BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS +
    BESTIE_THREAD_SUMMARIZE_IDLE_SETTLE_SECONDS;
  assert.equal(
    isBestieThreadSummarizePendingStale({
      agentWorkingOnBestieDm: false,
      nowSeconds: settleAt,
      pendingSummarize: {
        startedAt,
        threadId: "ch:root",
        triggerMessageId: "s1",
      },
    }),
    true,
  );
  assert.equal(
    isBestieThreadSummarizePendingStale({
      agentWorkingOnBestieDm: true,
      nowSeconds: startedAt + BESTIE_THREAD_SUMMARIZE_HARD_TIMEOUT_SECONDS,
      pendingSummarize: {
        startedAt,
        threadId: "ch:root",
        triggerMessageId: "s1",
      },
    }),
    true,
  );
});

test("competing coffee abandons summarize pending after grace when idle", () => {
  const startedAt = 1_000;
  assert.equal(
    isBestieThreadSummarizePendingStale({
      agentWorkingOnBestieDm: true,
      nowSeconds: startedAt + BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS + 2,
      latestCompetingTriggerAt: startedAt + 5,
      pendingSummarize: {
        startedAt,
        threadId: "ch:root",
        triggerMessageId: "s1",
      },
    }),
    false,
  );
  assert.equal(
    isBestieThreadSummarizePendingStale({
      agentWorkingOnBestieDm: false,
      nowSeconds: startedAt + BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS + 2,
      latestCompetingTriggerAt: startedAt + 5,
      pendingSummarize: {
        startedAt,
        threadId: "ch:root",
        triggerMessageId: "s1",
      },
    }),
    true,
  );
});
