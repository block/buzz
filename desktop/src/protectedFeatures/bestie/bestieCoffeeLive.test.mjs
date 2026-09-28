import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_COFFEE_HARD_TIMEOUT_SECONDS,
  BESTIE_COFFEE_IDLE_SETTLE_SECONDS,
  BESTIE_COFFEE_START_GRACE_SECONDS,
  isBestieCoffeeLive,
  isBestieCoffeePendingStale,
  messageLooksLikeBestieCoffeeTrigger,
  messageLooksLikeBestieCompetingSystemTrigger,
  replyParentIdFromEventTags,
  shouldDisableBestieCoffeeBrew,
} from "./bestieCoffeeLive.ts";

test("messageLooksLikeBestieCoffeeTrigger matches marker and slash skill", () => {
  assert.equal(
    messageLooksLikeBestieCoffeeTrigger("[Bestie coffee]\n\n/hula-coffee"),
    true,
  );
  assert.equal(messageLooksLikeBestieCoffeeTrigger("/hula-coffee force"), true);
  assert.equal(messageLooksLikeBestieCoffeeTrigger("please brew coffee"), false);
});

test("live during start grace even without ACP working (👀 phase)", () => {
  const startedAt = 1_000;
  assert.equal(
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: false,
      nowSeconds: startedAt + BESTIE_COFFEE_START_GRACE_SECONDS,
      pendingRun: { source: "brew", startedAt, triggerMessageId: null },
    }),
    true,
  );
  assert.equal(
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: false,
      nowSeconds: startedAt + BESTIE_COFFEE_START_GRACE_SECONDS + 1,
      pendingRun: { source: "brew", startedAt, triggerMessageId: null },
    }),
    false,
  );
});

test("live while ACP agent is working the Bestie DM", () => {
  assert.equal(
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: true,
      nowSeconds: 5_000,
      pendingRun: {
        source: "scheduled",
        startedAt: 1_000,
        triggerMessageId: "t1",
      },
    }),
    true,
  );
});

test("manual coffee trigger + ACP working disables brew without pendingRun", () => {
  assert.equal(
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: true,
      nowSeconds: 2_000,
      openCoffeeTriggerAt: 1_500,
      pendingRun: null,
    }),
    true,
  );
  assert.equal(
    shouldDisableBestieCoffeeBrew({
      agentWorkingOnBestieDm: true,
      nowSeconds: 2_000,
      openCoffeeTriggerAt: 1_500,
      pendingRun: null,
    }),
    true,
  );
});

test("stale pending clears after idle past grace + settle", () => {
  const startedAt = 1_000;
  const settleAt =
    startedAt +
    BESTIE_COFFEE_START_GRACE_SECONDS +
    BESTIE_COFFEE_IDLE_SETTLE_SECONDS;
  assert.equal(
    isBestieCoffeePendingStale({
      agentWorkingOnBestieDm: false,
      nowSeconds: startedAt + BESTIE_COFFEE_START_GRACE_SECONDS + 1,
      pendingRun: { source: "brew", startedAt, triggerMessageId: null },
    }),
    false,
  );
  assert.equal(
    isBestieCoffeePendingStale({
      agentWorkingOnBestieDm: false,
      nowSeconds: settleAt,
      pendingRun: { source: "brew", startedAt, triggerMessageId: null },
    }),
    true,
  );
  assert.equal(
    isBestieCoffeePendingStale({
      agentWorkingOnBestieDm: true,
      nowSeconds: settleAt,
      pendingRun: { source: "brew", startedAt, triggerMessageId: null },
    }),
    false,
  );
});

test("hard timeout clears pending even while ACP working signal stuck", () => {
  const startedAt = 1_000;
  assert.equal(
    isBestieCoffeePendingStale({
      agentWorkingOnBestieDm: true,
      nowSeconds: startedAt + BESTIE_COFFEE_HARD_TIMEOUT_SECONDS,
      pendingRun: { source: "brew", startedAt, triggerMessageId: "t1" },
    }),
    true,
  );
});

test("newer competing system turn abandons coffee pending after grace when idle", () => {
  const startedAt = 1_000;
  assert.equal(
    isBestieCoffeePendingStale({
      agentWorkingOnBestieDm: true,
      nowSeconds: startedAt + BESTIE_COFFEE_START_GRACE_SECONDS + 1,
      latestCompetingTriggerAt: startedAt + 10,
      pendingRun: { source: "brew", startedAt, triggerMessageId: "t1" },
    }),
    false,
  );
  assert.equal(
    isBestieCoffeePendingStale({
      agentWorkingOnBestieDm: false,
      nowSeconds: startedAt + BESTIE_COFFEE_START_GRACE_SECONDS + 1,
      latestCompetingTriggerAt: startedAt + 10,
      pendingRun: { source: "brew", startedAt, triggerMessageId: "t1" },
    }),
    true,
  );
});

test("competing system triggers match summarize / job / reminder", () => {
  assert.equal(
    messageLooksLikeBestieCompetingSystemTrigger(
      "[Bestie thread summarize]\n\nPlease summarize…",
    ),
    true,
  );
  assert.equal(
    messageLooksLikeBestieCompetingSystemTrigger("[Bestie job: inbox]\n\nok"),
    true,
  );
  assert.equal(
    messageLooksLikeBestieCompetingSystemTrigger("[Bestie reminder]\n\nDue"),
    true,
  );
  assert.equal(
    messageLooksLikeBestieCompetingSystemTrigger("[Bestie coffee]\n\n/hula-coffee"),
    false,
  );
});

test("not live when Thread Summarize is newer than coffee pending", () => {
  assert.equal(
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: true,
      nowSeconds: 5_000,
      latestCompetingTriggerAt: 4_000,
      pendingRun: {
        source: "brew",
        startedAt: 1_000,
        triggerMessageId: "coffee-1",
      },
    }),
    false,
  );
  assert.equal(
    shouldDisableBestieCoffeeBrew({
      agentWorkingOnBestieDm: true,
      nowSeconds: 5_000,
      latestCompetingTriggerAt: 4_000,
      pendingRun: {
        source: "brew",
        startedAt: 1_000,
        triggerMessageId: "coffee-1",
      },
    }),
    false,
  );
});

test("open coffee trigger + working is not live after newer summarize", () => {
  assert.equal(
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: true,
      nowSeconds: 3_000,
      openCoffeeTriggerAt: 1_500,
      latestCompetingTriggerAt: 2_500,
      pendingRun: null,
    }),
    false,
  );
});

test("replyParentIdFromEventTags prefers reply marker", () => {
  assert.equal(
    replyParentIdFromEventTags([
      ["e", "root-id", "", "root"],
      ["e", "parent-id", "", "reply"],
    ]),
    "parent-id",
  );
  assert.equal(replyParentIdFromEventTags([["e", "only-e"]]), "only-e");
  assert.equal(replyParentIdFromEventTags([]), null);
});
