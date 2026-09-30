import assert from "node:assert/strict";
import test from "node:test";

import {
  collectThinkingContentItems,
  collectTurnPromptContext,
  findNearestTurnIdByTime,
  findTurnIdForPromptEvent,
  formatTurnDuration,
  formatTurnTokens,
  MAX_RELIABLE_TURN_DURATION_SEC,
  normalizeUnixSeconds,
  resolveTurnDurationSeconds,
} from "./agentMessageTurnMeta.ts";

test("formatTurnDuration", () => {
  assert.equal(formatTurnDuration(null), null);
  assert.equal(formatTurnDuration(12), "12s");
  assert.equal(formatTurnDuration(65), "1:05");
  assert.equal(formatTurnDuration(600), "10:00");
});

test("formatTurnTokens", () => {
  assert.equal(formatTurnTokens(null), null);
  assert.equal(formatTurnTokens(42), "42");
  assert.equal(formatTurnTokens("1500"), "1.5k");
  assert.equal(formatTurnTokens("2000000"), "2.0M");
});

test("findTurnIdForPromptEvent matches triggeringEventIds", () => {
  const events = [
    {
      kind: "turn_started",
      turnId: "turn-a",
      payload: { triggeringEventIds: ["PROMPT1"] },
    },
  ];
  assert.equal(findTurnIdForPromptEvent(events, "prompt1"), "turn-a");
  assert.equal(findTurnIdForPromptEvent(events, "other"), null);
});

test("findNearestTurnIdByTime prefers turn with thinking content", () => {
  const items = [
    {
      id: "1",
      type: "lifecycle",
      timestamp: "2026-01-01T00:00:10Z",
      turnId: "t-life",
    },
    {
      id: "2",
      type: "thought",
      text: "hmm",
      timestamp: "2026-01-01T00:00:12Z",
      turnId: "t-think",
    },
  ];
  assert.equal(
    findNearestTurnIdByTime(items, Date.parse("2026-01-01T00:00:11Z") / 1000),
    "t-think",
  );
});

test("collectThinkingContentItems keeps thought/tool and orphans in window", () => {
  const items = [
    {
      id: "life",
      type: "lifecycle",
      timestamp: "2026-01-01T00:00:10Z",
      turnId: "t1",
    },
    {
      id: "th",
      type: "thought",
      text: "plan",
      timestamp: "2026-01-01T00:00:11Z",
      turnId: "t1",
    },
    {
      id: "orphan",
      type: "tool",
      title: "shell",
      timestamp: "2026-01-01T00:00:12Z",
      turnId: null,
    },
  ];
  const collected = collectThinkingContentItems(items, {
    turnId: "t1",
    windowStartSec: Date.parse("2026-01-01T00:00:00Z") / 1000,
    windowEndSec: Date.parse("2026-01-01T00:00:20Z") / 1000,
  });
  assert.deepEqual(collected.map((i) => i.id).sort(), ["orphan", "th"]);
});

test("normalizeUnixSeconds coerces ms", () => {
  assert.equal(normalizeUnixSeconds(1_700_000_000), 1_700_000_000);
  assert.equal(normalizeUnixSeconds(1_700_000_000_000), 1_700_000_000);
  assert.equal(normalizeUnixSeconds(null), null);
});

test("resolveTurnDurationSeconds prefers prompt; falls back to long spans", () => {
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_100,
      turnStartedAtSec: 1_000_080,
      promptCreatedAtSec: 1_000_090,
    }),
    10,
  );
  // Prefer reliable turn_started over an absurd parent span.
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + 680,
      turnStartedAtSec: 1_000_000 + 670,
      promptCreatedAtSec: 1_000_000,
    }),
    10,
  );
  // Long-only candidate still returns a value so the chip is never empty.
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + 680,
      turnStartedAtSec: null,
      promptCreatedAtSec: 1_000_000,
    }),
    680,
  );
  assert.ok(680 > MAX_RELIABLE_TURN_DURATION_SEC);
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_045,
      turnStartedAtSec: 1_000_000,
      promptCreatedAtSec: null,
    }),
    45,
  );
  // Mid-range thread ancestor (still "reliable") must not beat turn_started.
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + 10,
      turnStartedAtSec: 1_000_000,
      promptCreatedAtSec: 1_000_000 - 180,
    }),
    10,
  );
});

test("collectTurnPromptContext joins prompt user + context for turn", () => {
  const items = [
    {
      id: "u1",
      type: "message",
      role: "user",
      acpSource: "session/prompt:user",
      turnId: "t1",
      timestamp: "2026-09-30T00:00:00.000Z",
      text: "Are you there?",
      title: "You",
    },
    {
      id: "c1",
      type: "metadata",
      acpSource: "session/prompt:context",
      turnId: "t1",
      timestamp: "2026-09-30T00:00:00.000Z",
      sections: [{ title: "Channel", body: "#general" }],
    },
    {
      id: "l1",
      type: "lifecycle",
      acpSource: "turn_started",
      turnId: "t1",
      timestamp: "2026-09-30T00:00:01.000Z",
      title: "Turn started",
      text: "",
    },
  ];
  const result = collectTurnPromptContext(items, {
    turnId: "t1",
    windowStartSec: 1_000_000,
    windowEndSec: 1_000_100,
  });
  assert.equal(result.hasContext, true);
  assert.equal(result.sections[0].body, "Are you there?");
  assert.equal(result.sections[1].title, "Channel");
  assert.equal(result.setup.length, 1);
});
