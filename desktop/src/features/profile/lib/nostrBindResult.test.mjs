import assert from "node:assert/strict";
import test from "node:test";

import {
  getNostrBindResultPresentation,
  hasNostrBindResultAcknowledgement,
  observeNostrBindResult,
} from "./nostrBindResult.ts";

const baseOptions = {
  resultUrl: "https://api.run402.com/buzz-human-adoption-results/v1",
  ownerProofEvent: { id: "signed-event" },
  expiresAt: "2099-01-01T00:00:00Z",
};

function clock(start = 1_000) {
  let now = start;
  return {
    now: () => now,
    sleep: async (milliseconds) => {
      now += milliseconds;
    },
  };
}

test("observes pending until the exact approval is authoritatively completed", async () => {
  const states = [];
  const reads = [{ status: "pending" }, { status: "completed" }];
  const result = await observeNostrBindResult({
    ...baseOptions,
    ...clock(),
    isActive: () => true,
    onState: (state) => states.push(state),
    readResult: async () => reads.shift(),
  });

  assert.deepEqual(states, [{ status: "waiting" }, { status: "completed" }]);
  assert.deepEqual(result, { status: "completed" });
});

test("keeps explicit action-required, expired, and cancelled outcomes open", async () => {
  for (const response of [
    {
      status: "action_required",
      resultCode: "BUZZ_OWNER_PROOF_INVALID",
    },
    { status: "expired" },
    { status: "cancelled" },
  ]) {
    const result = await observeNostrBindResult({
      ...baseOptions,
      ...clock(),
      isActive: () => true,
      onState: () => {},
      readResult: async () => response,
    });
    assert.deepEqual(result, response);
  }
});

test("retries transient reachability failures within a bounded observation window", async () => {
  let attempts = 0;
  const result = await observeNostrBindResult({
    ...baseOptions,
    ...clock(),
    isActive: () => true,
    onState: () => {},
    readResult: async () => {
      attempts += 1;
      if (attempts === 1) {
        throw { kind: "unreachable" };
      }
      return { status: "completed" };
    },
  });

  assert.equal(attempts, 2);
  assert.deepEqual(result, { status: "completed" });
});

test("reports uncertainty rather than success when the endpoint remains unreachable", async () => {
  let attempts = 0;
  const result = await observeNostrBindResult({
    ...baseOptions,
    ...clock(),
    maxObservationMs: 2_000,
    pollIntervalMs: 1_000,
    isActive: () => true,
    onState: () => {},
    readResult: async () => {
      attempts += 1;
      throw { kind: "unreachable" };
    },
  });

  assert.equal(attempts, 2);
  assert.deepEqual(result, {
    status: "unable_to_confirm",
    reason: "unreachable",
  });
});

test("distinguishes a reachable endpoint that never returns a final result", async () => {
  const result = await observeNostrBindResult({
    ...baseOptions,
    ...clock(),
    maxObservationMs: 2_000,
    pollIntervalMs: 1_000,
    isActive: () => true,
    onState: () => {},
    readResult: async () => ({ status: "pending" }),
  });

  assert.deepEqual(result, {
    status: "unable_to_confirm",
    reason: "no_final_result",
  });
  assert.match(
    getNostrBindResultPresentation(result).description,
    /did not return a final result/,
  );
});

test("fails closed on malformed or non-allowlisted native responses", async () => {
  for (const response of [
    { status: "completed", extra: true },
    { status: "action_required", resultCode: "INTERNAL_ERROR" },
    { status: "future_status" },
  ]) {
    const result = await observeNostrBindResult({
      ...baseOptions,
      ...clock(),
      isActive: () => true,
      onState: () => {},
      readResult: async () => response,
    });
    assert.deepEqual(result, {
      status: "unable_to_confirm",
      reason: "invalid_response",
    });
  }
});

test("ignores a completed response from a superseded deep-link request", async () => {
  let active = true;
  let releaseRead;
  const states = [];
  const read = new Promise((resolve) => {
    releaseRead = resolve;
  });
  const observation = observeNostrBindResult({
    ...baseOptions,
    ...clock(),
    isActive: () => active,
    onState: (state) => states.push(state),
    readResult: async () => read,
  });

  active = false;
  releaseRead({ status: "completed" });
  const result = await observation;

  assert.deepEqual(result, { status: "superseded" });
  assert.deepEqual(states, [{ status: "waiting" }]);
});

test("uses the released legacy flow unless both recognized result fields exist", () => {
  assert.equal(hasNostrBindResultAcknowledgement({}), false);
  assert.equal(
    hasNostrBindResultAcknowledgement({
      resultProtocol: "run402_adoption_result_v1",
    }),
    false,
  );
  assert.equal(
    hasNostrBindResultAcknowledgement({
      resultProtocol: "future_protocol",
      resultUrl: "https://api.run402.com/result",
    }),
    false,
  );
  assert.equal(
    hasNostrBindResultAcknowledgement({
      resultProtocol: "run402_adoption_result_v1",
      resultUrl: "https://api.run402.com/result",
    }),
    true,
  );
});

test("gives every terminal outcome an explicit status and next action", () => {
  const states = [
    { status: "waiting" },
    { status: "completed" },
    { status: "expired" },
    { status: "cancelled" },
    { status: "unable_to_confirm", reason: "no_final_result" },
    { status: "unable_to_confirm", reason: "unreachable" },
    { status: "unable_to_confirm", reason: "invalid_response" },
    { status: "unable_to_confirm", reason: "unsafe_endpoint" },
    ...[
      "BUZZ_ADOPTION_CALLBACK_INVALID",
      "BUZZ_ADOPTION_OFFER_NOT_ACTIVE",
      "BUZZ_ADOPTION_TARGET_MISMATCH",
      "BUZZ_IDEMPOTENCY_CONFLICT",
      "BUZZ_OWNER_PROOF_INVALID",
      "STEP_UP_REQUIRED",
      "BUZZ_ADOPTION_COMPLETION_REJECTED",
    ].map((resultCode) => ({ status: "action_required", resultCode })),
  ];

  for (const state of states) {
    const presentation = getNostrBindResultPresentation(state);
    assert.ok(presentation.title.length > 0);
    assert.ok(presentation.description.length > 0);
    assert.ok(presentation.nextAction.length > 0);
    assert.ok(presentation.closeLabel.length > 0);
  }
});
