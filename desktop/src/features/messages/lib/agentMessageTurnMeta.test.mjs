import assert from "node:assert/strict";
import test from "node:test";

import {
  collectThinkingContentItems,
  collectTurnPromptContext,
  earliestContentStartedAtSec,
  findNearestTurnIdByTime,
  findTurnIdForPromptEvent,
  findTurnUsageUsedTokens,
  formatTurnDuration,
  formatTurnTokens,
  MAX_RELIABLE_TURN_DURATION_SEC,
  normalizeUnixSeconds,
  parseUsageTokensUsedLabel,
  resolveReplyChipTokenCount,
  resolveTurnDurationSeconds,
  selectPromptCreatedAtForDuration,
} from "./agentMessageTurnMeta.ts";

test("formatTurnDuration", () => {
  assert.equal(formatTurnDuration(null), null);
  assert.equal(formatTurnDuration(12), "12s");
  assert.equal(formatTurnDuration(65), "1:05");
  assert.equal(formatTurnDuration(600), "10:00");
  // m:ss with unbounded minutes — 11737s renders as the Captain bug chip.
  assert.equal(formatTurnDuration(195 * 60 + 37), "195:37");
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

test("findNearestTurnIdByTime prefers short turn ending at reply over long prior turn", () => {
  // Prior long turn emits a late tool beside the reply; old nearest-item
  // scoring stole the join → 19:47 duration + wrong 44200 tokens.
  const replySec = Date.parse("2026-09-30T08:44:00Z") / 1000;
  const items = [
    {
      id: "old-start",
      type: "thought",
      text: "earlier",
      timestamp: "2026-09-30T08:24:20Z",
      turnId: "turn-long",
    },
    {
      id: "old-late",
      type: "tool",
      title: "shell",
      timestamp: "2026-09-30T08:43:58Z",
      turnId: "turn-long",
    },
    {
      id: "new-start",
      type: "thought",
      text: "now",
      timestamp: "2026-09-30T08:43:05Z",
      turnId: "turn-short",
    },
    {
      id: "new-end",
      type: "tool",
      title: "reply",
      timestamp: "2026-09-30T08:43:55Z",
      turnId: "turn-short",
    },
  ];
  assert.equal(findNearestTurnIdByTime(items, replySec), "turn-short");
});

test("findNearestTurnIdByTime does not let a later turn steal an older reply", () => {
  // Newer turn starts ~15s after the older reply. Old shortest-span scoring
  // gave it span≈0 under a 30s post-start grace and rebound every chip to the
  // latest usage total (1.1M → 243.9k on all Opus rows).
  const olderReplySec = Date.parse("2026-09-30T09:40:00Z") / 1000;
  const items = [
    {
      id: "old-start",
      type: "thought",
      text: "older turn",
      timestamp: "2026-09-30T09:39:50Z",
      turnId: "turn-older",
    },
    {
      id: "old-end",
      type: "tool",
      title: "reply",
      timestamp: "2026-09-30T09:39:59Z",
      turnId: "turn-older",
    },
    {
      id: "new-start",
      type: "thought",
      text: "newer turn",
      timestamp: "2026-09-30T09:40:15Z",
      turnId: "turn-newer",
    },
    {
      id: "new-usage",
      type: "tool",
      title: "usage",
      timestamp: "2026-09-30T09:40:20Z",
      turnId: "turn-newer",
    },
  ];
  assert.equal(findNearestTurnIdByTime(items, olderReplySec), "turn-older");
});

test("parseUsageTokensUsedLabel reads Usage Tokens numerator", () => {
  assert.equal(
    parseUsageTokensUsedLabel("Tokens: 81645/1000000 ($0.9677 USD)"),
    81645,
  );
  assert.equal(parseUsageTokensUsedLabel("Tokens: 1500/8192"), 1500);
  assert.equal(parseUsageTokensUsedLabel("nope"), null);
});

test("findTurnUsageUsedTokens returns latest used for the turn only", () => {
  const events = [
    {
      kind: "acp_read",
      turnId: "turn-a",
      timestamp: "2026-09-30T09:40:00Z",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "usage_update",
            used: 1000,
            size: 1_000_000,
          },
        },
      },
    },
    {
      kind: "acp_read",
      turnId: "turn-a",
      timestamp: "2026-09-30T09:40:05Z",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "usage_update",
            used: 81645,
            size: 1_000_000,
          },
        },
      },
    },
    {
      kind: "acp_read",
      turnId: "turn-b",
      timestamp: "2026-09-30T09:41:00Z",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "usage_update",
            used: 243900,
            size: 1_000_000,
          },
        },
      },
    },
  ];
  assert.equal(findTurnUsageUsedTokens(events, "turn-a"), 81645);
  assert.equal(findTurnUsageUsedTokens(events, "turn-b"), 243900);
  assert.equal(findTurnUsageUsedTokens(events, "turn-missing"), null);
});

test("resolveReplyChipTokenCount prefers usage used; ignores time-near 44200", () => {
  assert.equal(
    resolveReplyChipTokenCount({
      usageUsedTokens: 81645,
      metricMatchKind: "time",
      metricTurnTotalTokens: "243900",
    }),
    81645,
  );
  assert.equal(
    resolveReplyChipTokenCount({
      usageUsedTokens: null,
      metricMatchKind: "time",
      metricTurnTotalTokens: "243900",
    }),
    null,
  );
  assert.equal(
    resolveReplyChipTokenCount({
      usageUsedTokens: null,
      metricMatchKind: "exact",
      metricTurnTotalTokens: "1500",
    }),
    "1500",
  );
  assert.equal(
    formatTurnTokens(
      resolveReplyChipTokenCount({
        usageUsedTokens: 81645,
        metricMatchKind: null,
        metricTurnTotalTokens: null,
      }),
    ),
    "81.6k",
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
  // Bare parent/prompt outside the reliable cap is untrusted — omit the chip
  // rather than show multi-hour absurd durations (Captain `195:37` case).
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + 680,
      turnStartedAtSec: null,
      promptCreatedAtSec: 1_000_000,
    }),
    null,
  );
  assert.ok(680 > MAX_RELIABLE_TURN_DURATION_SEC);
  // Captain-shaped: ~3h thread ancestor, no turn_started → null (not 195:37).
  const captainSpan = 195 * 60 + 37;
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + captainSpan,
      turnStartedAtSec: null,
      promptCreatedAtSec: 1_000_000,
    }),
    null,
  );
  // Same case with thinking content near the reply → short duration.
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + captainSpan,
      turnStartedAtSec: null,
      promptCreatedAtSec: 1_000_000,
      contentStartedAtSec: 1_000_000 + captainSpan - 23,
    }),
    23,
  );
  // Trusted long tool turn (turn_started present) still shows.
  assert.equal(
    resolveTurnDurationSeconds({
      replyCreatedAt: 1_000_000 + 680,
      turnStartedAtSec: 1_000_000,
      promptCreatedAtSec: null,
    }),
    680,
  );
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

test("selectPromptCreatedAtForDuration ignores parent unless triggering-event", () => {
  assert.equal(
    selectPromptCreatedAtForDuration({
      joinMethod: "none",
      promptCreatedAtSec: null,
      parentCreatedAtSec: 1_000_000,
    }),
    null,
  );
  assert.equal(
    selectPromptCreatedAtForDuration({
      joinMethod: "time-proximity",
      promptCreatedAtSec: null,
      parentCreatedAtSec: 1_000_000,
    }),
    null,
  );
  assert.equal(
    selectPromptCreatedAtForDuration({
      joinMethod: "triggering-event",
      promptCreatedAtSec: null,
      parentCreatedAtSec: 1_000_000,
    }),
    1_000_000,
  );
  assert.equal(
    selectPromptCreatedAtForDuration({
      joinMethod: "none",
      promptCreatedAtSec: 1_000_050,
      parentCreatedAtSec: 1_000_000,
    }),
    1_000_050,
  );
});

test("earliestContentStartedAtSec picks first thought/tool", () => {
  assert.equal(
    earliestContentStartedAtSec([
      {
        id: "1",
        type: "lifecycle",
        timestamp: "2026-01-01T00:00:00Z",
        turnId: "t1",
      },
      {
        id: "2",
        type: "thought",
        text: "plan",
        timestamp: "2026-01-01T00:00:10Z",
        turnId: "t1",
      },
      {
        id: "3",
        type: "tool",
        title: "shell",
        timestamp: "2026-01-01T00:00:05Z",
        turnId: "t1",
      },
    ]),
    Date.parse("2026-01-01T00:00:05Z") / 1000,
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
