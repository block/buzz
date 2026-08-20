import assert from "node:assert/strict";
import test from "node:test";

import {
  account,
  classifyRequest,
  COMPLETE_COVERAGE,
  deriveObligation,
  REQUEST_KINDS,
} from "@/shared/lib/disposition";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { deriveChannelDispositionSummary } from "./dispositionSummary.ts";

/**
 * Adapter conformance: the desktop's per-request status and channel summary
 * must agree with the shared verifier on every request, field by field.
 *
 * **This test previously compared bucket sums, and that was not enough.**
 * Swapping the desktop meanings of `responded` and `errored` passed all seven
 * of its cases, because the mixed fixture had one of each and the assertion
 * only checked `respondedUnsettled + attemptFailed` against `open.length`. A
 * test built as the structural guarantee against cross-adapter divergence was
 * mutation-checked against the one defect it was designed for, and never
 * asked what else it should catch.
 *
 * So the comparison is now per-request and total: for every marked event, the
 * classification, outcome kind AND state, latest observation, warnings,
 * reason, and resolved flag must match an oracle built directly from the
 * shared verifier. Aggregates are checked too, but they are the weaker claim.
 */

const CHANNEL = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);
const HUMAN = "c".repeat(64);
const REQUESTER = "d".repeat(64);
const KIND_AGENT_DISPOSITION = 44300;

let seq = 0;
const nextId = () => String(++seq).padStart(64, "0");

function request(tags, id = nextId()) {
  return {
    id,
    pubkey: REQUESTER,
    kind: REQUEST_KINDS[0],
    created_at: 100,
    content: "@agent do the thing",
    tags,
    sig: "",
  };
}

function disposition(requestId, signer, state, createdAt = 200, reason) {
  return {
    id: nextId(),
    pubkey: signer,
    kind: KIND_AGENT_DISPOSITION,
    created_at: createdAt,
    content: JSON.stringify({
      disposition: state,
      reason: reason ?? `r-${state}`,
    }),
    tags: [
      ["e", requestId],
      ["h", CHANNEL],
      ["p", REQUESTER],
      ["disposition", state],
    ],
    sig: "",
  };
}

const validTags = (agent = AGENT) => [
  ["h", CHANNEL],
  ["t", "request"],
  ["agent", agent],
  ["p", agent],
];

/**
 * The oracle: what every marked event's status must be, computed straight
 * from the shared verifier with no desktop code involved.
 */
function oracle(events) {
  const byRequest = new Map();
  for (const e of events) {
    if (e.kind !== KIND_AGENT_DISPOSITION) continue;
    const target = e.tags.find((t) => t[0] === "e")?.[1];
    if (target == null) continue;
    byRequest.set(target, [...(byRequest.get(target) ?? []), e]);
  }

  const expected = new Map();
  for (const event of events) {
    const classified = classifyRequest(event);
    switch (classified.kind) {
      case "not_request":
        break;
      case "invalid":
      case "unsupported":
        expected.set(event.id, {
          kind: classified.kind,
          reason: classified.reason,
        });
        break;
      case "valid": {
        const derived = deriveObligation(
          classified.obligation,
          byRequest.get(event.id) ?? [],
        );
        expected.set(event.id, {
          kind: "valid",
          outcome: derived.outcome,
          latestObservation: derived.latestObservation,
          reason: derived.reason,
          warnings: derived.warnings,
          resolved: derived.outcome.kind === "settled",
        });
        break;
      }
    }
  }
  return expected;
}

/** Run one event set through both paths and compare every field. */
function compare(events, label) {
  const expected = oracle(events);
  const shared = account(events, events, COMPLETE_COVERAGE);

  const messages = formatTimelineMessages(
    events,
    { id: CHANNEL, name: "general", channelType: "channel" },
    REQUESTER,
    null,
  );
  const summary = deriveChannelDispositionSummary(
    messages.map((message) => ({ message })),
  );

  // --- per-request equivalence, which is the real claim ---
  const actual = new Map(
    messages
      .filter((m) => m.requestStatus != null)
      .map((m) => [m.id, m.requestStatus]),
  );
  assert.deepEqual(
    [...actual.keys()].sort(),
    [...expected.keys()].sort(),
    `${label}: the desktop and the verifier disagree about WHICH events carry a status`,
  );
  for (const [id, want] of expected) {
    assert.deepEqual(
      actual.get(id),
      want,
      `${label}: request ${id} differs from the shared verifier`,
    );
  }

  // --- aggregates, the weaker claim, kept as a second net ---
  assert.equal(summary.settled, shared.settled.length, `${label}: settled`);
  assert.equal(
    summary.noRecord.length,
    shared.unanswered.length,
    `${label}: unanswered — the bucket a client fault used to leak into`,
  );
  assert.equal(summary.disputed, shared.disputed.length, `${label}: disputed`);
  assert.equal(
    summary.invalidOrUnsupported,
    shared.invalidRequests.length + shared.unsupportedRequests.length,
    `${label}: invalid/unsupported`,
  );
  // Split, not summed: summing these is exactly what let a responded/errored
  // swap pass.
  assert.equal(
    summary.respondedUnsettled,
    shared.open.filter((id) => expected.get(id)?.outcome?.state === "responded")
      .length,
    `${label}: responded`,
  );
  assert.equal(
    summary.attemptFailed,
    shared.open.filter((id) => expected.get(id)?.outcome?.state === "errored")
      .length,
    `${label}: errored`,
  );
  return { summary, shared, expected };
}

test("settled, open, and unanswered obligations agree per request", () => {
  const settled = request(validTags());
  const responded = request(validTags());
  const gap = request(validTags());
  compare(
    [
      settled,
      responded,
      gap,
      disposition(settled.id, AGENT, "completed"),
      disposition(responded.id, AGENT, "responded"),
    ],
    "mixed",
  );
});

test("responded and errored are distinguished, not summed", () => {
  // The mutation the old aggregate test could not see: swapping the desktop
  // meanings of these two states.
  //
  // **The counts here are deliberately asymmetric (2 vs 1).** A one-of-each
  // fixture cannot detect a swap — both counts stay 1 — which is precisely
  // why the previous version of this file passed with the states reversed.
  // A test whose fixture is symmetric in the dimension under test proves
  // nothing about that dimension.
  const a = request(validTags());
  const b = request(validTags());
  const c = request(validTags());
  const { summary } = compare(
    [
      a,
      b,
      c,
      disposition(a.id, AGENT, "responded"),
      disposition(b.id, AGENT, "responded"),
      disposition(c.id, AGENT, "errored"),
    ],
    "responded vs errored",
  );
  assert.equal(summary.respondedUnsettled, 2);
  assert.equal(summary.attemptFailed, 1);
});

test("an invalid request is never counted as an agent gap by either path", () => {
  const targetless = request([
    ["h", CHANNEL],
    ["t", "request"],
    ["p", HUMAN],
  ]);
  const { summary, shared } = compare([targetless], "targetless");
  assert.equal(summary.noRecord.length, 0);
  assert.equal(summary.invalidOrUnsupported, 1);
  assert.equal(shared.invalidRequests.length, 1);
});

test("an unsupported multi-target request agrees per request", () => {
  const multi = request([
    ["h", CHANNEL],
    ["t", "request"],
    ["agent", AGENT],
    ["agent", OTHER_AGENT],
    ["p", AGENT],
    ["p", OTHER_AGENT],
  ]);
  const { summary } = compare([multi], "multi-target");
  assert.equal(summary.noRecord.length, 0);
  assert.equal(summary.invalidOrUnsupported, 1);
});

test("a spoofed disposition leaves the obligation unanswered in both paths", () => {
  const req = request(validTags());
  const { summary } = compare(
    [req, disposition(req.id, HUMAN, "completed")],
    "spoof",
  );
  assert.equal(summary.noRecord.length, 1);
  assert.equal(summary.settled, 0);
});

test("a disputed history agrees per request, including its reason", () => {
  const req = request(validTags());
  const { expected } = compare(
    [
      req,
      disposition(req.id, AGENT, "completed", 200),
      disposition(req.id, AGENT, "refused", 300, "changed course"),
    ],
    "disputed",
  );
  assert.equal(expected.get(req.id).outcome.kind, "disputed");
});

test("a settled obligation stays settled under a later stray write", () => {
  const req = request(validTags());
  const { summary, expected } = compare(
    [
      req,
      disposition(req.id, AGENT, "completed", 200),
      disposition(req.id, AGENT, "errored", 300),
    ],
    "absorbed",
  );
  assert.equal(summary.settled, 1);
  assert.equal(summary.attemptFailed, 0);
  // The warning and the raw observation must both survive to the UI.
  assert.deepEqual(expected.get(req.id).warnings, ["ordered_after_terminal"]);
  assert.equal(expected.get(req.id).latestObservation, "errored");
});

test("a structurally invalid disposition never reaches either path", () => {
  // The validity boundary: an event the relay would reject must not settle
  // anything here either.
  const req = request(validTags());
  const doubleE = disposition(req.id, AGENT, "completed");
  doubleE.tags.push(["e", "9".repeat(64)]);
  const { summary } = compare([req, doubleE], "invalid disposition");
  assert.equal(summary.settled, 0);
  assert.equal(summary.noRecord.length, 1);
});

test("every bucket at once still agrees, field by field", () => {
  const settled = request(validTags());
  const responded = request(validTags());
  const responded2 = request(validTags());
  const errored = request(validTags());
  const gap = request(validTags());
  const disputed = request(validTags());
  const invalid = request([
    ["h", CHANNEL],
    ["t", "request"],
    ["p", HUMAN],
  ]);
  const unsupported = request([
    ["h", CHANNEL],
    ["t", "request"],
    ["agent", AGENT],
    ["agent", OTHER_AGENT],
    ["p", AGENT],
    ["p", OTHER_AGENT],
  ]);

  const { summary } = compare(
    [
      settled,
      responded,
      responded2,
      errored,
      gap,
      disputed,
      invalid,
      unsupported,
      disposition(settled.id, AGENT, "completed"),
      disposition(responded.id, AGENT, "responded"),
      disposition(responded2.id, AGENT, "responded"),
      disposition(errored.id, AGENT, "errored"),
      disposition(disputed.id, AGENT, "completed", 200),
      disposition(disputed.id, AGENT, "refused", 300),
    ],
    "all buckets",
  );

  // Asymmetric on purpose — see the responded/errored test above.
  assert.equal(summary.total, 8);
  assert.equal(summary.settled, 1);
  assert.equal(summary.respondedUnsettled, 2);
  assert.equal(summary.attemptFailed, 1);
  assert.equal(summary.noRecord.length, 1);
  assert.equal(summary.disputed, 1);
  assert.equal(summary.invalidOrUnsupported, 2);
  assert.equal(summary.needsAttention, 7);
});
