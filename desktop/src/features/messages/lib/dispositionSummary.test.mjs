import assert from "node:assert/strict";
import test from "node:test";

import { deriveChannelDispositionSummary } from "./dispositionSummary.ts";

/**
 * Unit tests for the tally itself. The classification these read comes from
 * the shared verifier upstream — see `dispositionSummary.adapter.test.mjs`
 * for the test that pins this tally against `account()` on real events, which
 * is what stops the two from drifting.
 */

function valid(id, outcome, extra = {}) {
  return {
    id,
    body: `request ${id}`,
    requestStatus: {
      kind: "valid",
      outcome,
      latestObservation: outcome.state ?? null,
      reason: "",
      warnings: [],
      resolved: outcome.kind === "settled",
      ...extra,
    },
  };
}

function unclassified(id) {
  return { id, body: `ordinary ${id}` };
}

test("counts only messages carrying a request status", () => {
  const summary = deriveChannelDispositionSummary([
    { message: valid("r1", { kind: "unanswered" }) },
    { message: unclassified("o1") },
  ]);
  assert.equal(summary.total, 1);
});

test("a valid obligation with nothing bound is a real gap", () => {
  const summary = deriveChannelDispositionSummary([
    { message: valid("r1", { kind: "unanswered" }) },
  ]);
  assert.deepEqual(summary.noRecord, [{ id: "r1", body: "request r1" }]);
  assert.equal(summary.settled, 0);
  assert.equal(summary.needsAttention, 1);
});

test("settled is the only done count", () => {
  const summary = deriveChannelDispositionSummary([
    { message: valid("r1", { kind: "settled", state: "completed" }) },
    { message: valid("r2", { kind: "settled", state: "refused" }) },
  ]);
  assert.equal(summary.settled, 2);
  assert.equal(summary.needsAttention, 0);
});

test("responded and errored are separate counts, and neither is settled", () => {
  // They used to share one `answered` count, so a channel whose every turn
  // failed rendered "all requests answered". "Has a record" and "was
  // answered" are different facts.
  const summary = deriveChannelDispositionSummary([
    { message: valid("r1", { kind: "open", state: "responded" }) },
    { message: valid("r2", { kind: "open", state: "errored" }) },
  ]);
  assert.equal(summary.respondedUnsettled, 1);
  assert.equal(summary.attemptFailed, 1);
  assert.equal(summary.settled, 0);
  assert.equal(summary.needsAttention, 2);
});

test("a disputed history is counted as disputed, never settled", () => {
  const summary = deriveChannelDispositionSummary([
    { message: valid("r1", { kind: "disputed" }) },
  ]);
  assert.equal(summary.disputed, 1);
  assert.equal(summary.settled, 0);
  assert.equal(summary.needsAttention, 1);
});

test("a warning does not move a settled obligation out of settled", () => {
  // The point of separating warnings from outcomes: a duplicate delivery is
  // redundant, not disputed, and must not read as an unresolved problem.
  const summary = deriveChannelDispositionSummary([
    {
      message: valid(
        "r1",
        { kind: "settled", state: "completed" },
        { warnings: ["duplicate_terminal"] },
      ),
    },
  ]);
  assert.equal(summary.settled, 1);
  assert.equal(summary.disputed, 0);
  assert.equal(summary.needsAttention, 0);
});

test("invalid and unsupported requests are their own bucket, never gaps", () => {
  const summary = deriveChannelDispositionSummary([
    {
      message: {
        id: "r1",
        body: "marked but targetless",
        requestStatus: { kind: "invalid", reason: "missing_agent_target" },
      },
    },
    {
      message: {
        id: "r2",
        body: "two agents",
        requestStatus: {
          kind: "unsupported",
          reason: "multiple_agent_targets",
        },
      },
    },
  ]);
  assert.equal(summary.invalidOrUnsupported, 2);
  assert.deepEqual(
    summary.noRecord,
    [],
    "a client fault must never be filed as an agent's unanswered request",
  );
  assert.equal(summary.needsAttention, 2);
});

test("an empty timeline yields an all-zero summary", () => {
  const summary = deriveChannelDispositionSummary([]);
  assert.deepEqual(summary, {
    total: 0,
    settled: 0,
    respondedUnsettled: 0,
    attemptFailed: 0,
    noRecord: [],
    disputed: 0,
    invalidOrUnsupported: 0,
    needsAttention: 0,
  });
});

test("every request lands in exactly one bucket", () => {
  const summary = deriveChannelDispositionSummary([
    { message: valid("r1", { kind: "settled", state: "completed" }) },
    { message: valid("r2", { kind: "open", state: "responded" }) },
    { message: valid("r3", { kind: "open", state: "errored" }) },
    { message: valid("r4", { kind: "unanswered" }) },
    { message: valid("r5", { kind: "disputed" }) },
    {
      message: {
        id: "r6",
        body: "bad",
        requestStatus: { kind: "invalid", reason: "missing_channel" },
      },
    },
  ]);
  const bucketed =
    summary.settled +
    summary.respondedUnsettled +
    summary.attemptFailed +
    summary.noRecord.length +
    summary.disputed +
    summary.invalidOrUnsupported;
  assert.equal(bucketed, summary.total, "buckets must partition the total");
  assert.equal(summary.needsAttention, summary.total - summary.settled);
});
