import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  aggregateAgentTurnMetrics,
  displayTokenTotal,
  EMPTY_USAGE_TOTALS,
  isTurnMetricPayload,
} from "./turnMetrics.ts";

function makePayload(overrides = {}) {
  return {
    harness: "goose",
    timestamp: "2026-07-29T12:00:00Z",
    sessionId: "sess-1",
    turnSeq: 1,
    ...overrides,
  };
}

describe("isTurnMetricPayload", () => {
  it("accepts a minimal payload (harness + timestamp only)", () => {
    assert.equal(
      isTurnMetricPayload({
        harness: "goose",
        timestamp: "2026-01-01T00:00:00Z",
      }),
      true,
    );
  });

  it("rejects payloads missing required fields", () => {
    assert.equal(isTurnMetricPayload(null), false);
    assert.equal(isTurnMetricPayload({}), false);
    assert.equal(isTurnMetricPayload({ harness: "", timestamp: "x" }), false);
    assert.equal(isTurnMetricPayload({ harness: "goose" }), false);
  });

  it("rejects negative or non-finite costUsd (NIP-AM numeric validity)", () => {
    assert.equal(
      isTurnMetricPayload(makePayload({ turn: { costUsd: -0.5 } })),
      false,
    );
    assert.equal(
      isTurnMetricPayload(
        makePayload({ cumulative: { costUsd: Number.POSITIVE_INFINITY } }),
      ),
      false,
    );
    assert.equal(
      isTurnMetricPayload(makePayload({ turn: { costUsd: 0.5 } })),
      true,
    );
  });

  it("ignores unknown fields (forward compatibility)", () => {
    assert.equal(
      isTurnMetricPayload(makePayload({ futureField: { nested: true } })),
      true,
    );
  });
});

describe("aggregateAgentTurnMetrics", () => {
  it("returns the empty totals for no payloads", () => {
    assert.deepEqual(aggregateAgentTurnMetrics([]), EMPTY_USAGE_TOTALS);
  });

  it("sums per-turn counts when no cumulative is reported", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({
        turnSeq: 1,
        turn: { inputTokens: 100, outputTokens: 10, costUsd: 0.01 },
      }),
      makePayload({
        turnSeq: 2,
        turn: { inputTokens: 200, outputTokens: 20, costUsd: 0.02 },
      }),
    ]);
    assert.equal(totals.turnCount, 2);
    assert.equal(totals.inputTokens, 300);
    assert.equal(totals.outputTokens, 30);
    assert.equal(totals.totalTokens, null);
    assert.ok(Math.abs(totals.costUsd - 0.03) < 1e-9);
  });

  it("prefers the latest cumulative over summed deltas within a session", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({
        turnSeq: 1,
        turn: { totalTokens: 500 },
        cumulative: { totalTokens: 500, costUsd: 0.05 },
      }),
      makePayload({
        turnSeq: 2,
        turn: { totalTokens: 700 },
        cumulative: { totalTokens: 1200, costUsd: 0.12 },
      }),
    ]);
    // Cumulative already covers both turns — summing deltas on top would
    // double-count (500 + 700 + 1200).
    assert.equal(totals.totalTokens, 1200);
    assert.ok(Math.abs(totals.costUsd - 0.12) < 1e-9);
    assert.equal(totals.turnCount, 2);
  });

  it("keeps the higher-turnSeq cumulative when events arrive out of order", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({ turnSeq: 3, cumulative: { totalTokens: 900 } }),
      makePayload({ turnSeq: 2, cumulative: { totalTokens: 600 } }),
    ]);
    assert.equal(totals.totalTokens, 900);
  });

  it("sums across independent sessions", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({ sessionId: "a", cumulative: { totalTokens: 100 } }),
      makePayload({ sessionId: "b", cumulative: { totalTokens: 200 } }),
      makePayload({ sessionId: null, turn: { totalTokens: 50 } }),
    ]);
    assert.equal(totals.totalTokens, 350);
  });

  it("treats sessionless payloads as independent groups", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({ sessionId: null, turn: { inputTokens: 10 } }),
      makePayload({ sessionId: null, turn: { inputTokens: 20 } }),
    ]);
    assert.equal(totals.inputTokens, 30);
  });

  it("skips unreliable deltas but keeps reliable ones", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({ turnSeq: 1, turn: { inputTokens: 100 } }),
      makePayload({
        turnSeq: 2,
        deltaReliable: false,
        turn: { inputTokens: 9999 },
      }),
    ]);
    assert.equal(totals.inputTokens, 100);
    assert.equal(totals.turnCount, 2);
  });

  it("keeps unreported fields null instead of coercing to 0", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({ turn: { inputTokens: 100 } }),
    ]);
    assert.equal(totals.inputTokens, 100);
    assert.equal(totals.outputTokens, null);
    assert.equal(totals.totalTokens, null);
    assert.equal(totals.costUsd, null);
  });

  it("tracks the newest turn timestamp", () => {
    const totals = aggregateAgentTurnMetrics([
      makePayload({ timestamp: "2026-07-29T12:00:00Z" }),
      makePayload({ timestamp: "2026-07-29T14:30:00Z" }),
      makePayload({ timestamp: "2026-07-29T09:00:00Z" }),
    ]);
    assert.equal(totals.lastTurnAt, Date.parse("2026-07-29T14:30:00Z"));
  });
});

describe("displayTokenTotal", () => {
  it("prefers the provider-reported total", () => {
    assert.equal(
      displayTokenTotal({
        ...EMPTY_USAGE_TOTALS,
        totalTokens: 1200,
        inputTokens: 100,
        outputTokens: 20,
      }),
      1200,
    );
  });

  it("falls back to input + output when no total was reported", () => {
    assert.equal(
      displayTokenTotal({
        ...EMPTY_USAGE_TOTALS,
        inputTokens: 100,
        outputTokens: 20,
      }),
      120,
    );
    assert.equal(
      displayTokenTotal({ ...EMPTY_USAGE_TOTALS, inputTokens: 100 }),
      100,
    );
  });

  it("returns null when nothing was reported", () => {
    assert.equal(displayTokenTotal(EMPTY_USAGE_TOTALS), null);
  });
});
