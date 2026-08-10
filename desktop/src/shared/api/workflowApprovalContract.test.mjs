import assert from "node:assert/strict";
import test from "node:test";

import { fromRawApproval } from "./tauriWorkflows.ts";

// The exact wire shape the relay's `approval_json` emits. If these two drift,
// the approval card silently renders `undefined` fields — the failure mode that
// shipped once already, where every field of the approval action response was
// undefined and the UI still looked like it worked.
const HERMES_PACKAGE = `BUZZ_REENGAGEMENT_DRAFT_READY
COMPANY: Social Hotspot
RELATIONSHIP: former client
LAST_CONTACTED_AT: 2026-03-14T09:12:00Z
CONTACT: Jamie Reed
LINKEDIN_TARGET: conv_8891244
THREAD_GIST: We paused the rollout in March pending their venue refit.
DRAFT_START
Hi Jamie, the refit should be done by now. Worth 20 minutes to pick the rollout back up? https://cal.com/nocoded/30min
DRAFT_END`;

function rawApproval(overrides = {}) {
  return {
    // Hex-encoded SHA-256 of the raw token. The desktop never sees the raw
    // bearer token; this value goes straight into the grant's `d` tag.
    token: "b".repeat(64),
    workflow_id: "65d30773-bea4-4f79-a4c9-b008f7902b91",
    run_id: "adc3ad0d-d487-4bed-acd7-31e676b50ce5",
    step_id: "gate",
    step_index: 0,
    approver_spec:
      "1a99c7e0596b98299393c384a3b1959374e483c6658772ce3337ea0474e74b90",
    status: "pending",
    approver_pubkey: null,
    note: null,
    request_message: HERMES_PACKAGE,
    expires_at: "2026-08-08T00:00:00+00:00",
    created_at: 1785800000,
    ...overrides,
  };
}

test("the exact Hermes package round-trips to the approval card", () => {
  const approval = fromRawApproval(rawApproval());

  // Byte-for-byte. The gate authorises only the text between the markers, so
  // any normalisation here would mean approving something other than what is
  // sent.
  assert.equal(approval.requestMessage, HERMES_PACKAGE);
  assert.ok(approval.requestMessage.includes("DRAFT_START"));
  assert.ok(approval.requestMessage.includes("DRAFT_END"));
  assert.ok(approval.requestMessage.includes("LINKEDIN_TARGET: conv_8891244"));
  assert.ok(approval.requestMessage.includes("Social Hotspot"));
});

test("request message is distinct from the approver note", () => {
  const approval = fromRawApproval(
    rawApproval({ note: "looks good, send it" }),
  );
  assert.equal(approval.note, "looks good, send it");
  assert.equal(approval.requestMessage, HERMES_PACKAGE);
  assert.notEqual(approval.note, approval.requestMessage);
});

test("a gate with no recorded package maps to null, not empty string", () => {
  // Rows predating migration 0027. The card must be able to tell "nothing was
  // recorded" apart from "the package was empty" so it can disable Approve
  // rather than render a blank box that looks reviewed.
  const approval = fromRawApproval(rawApproval({ request_message: null }));
  assert.equal(approval.requestMessage, null);
});

test("a missing request_message key does not become undefined", () => {
  const raw = rawApproval();
  delete raw.request_message;
  const approval = fromRawApproval(raw);
  // `undefined` would slip past a `!approval.requestMessage` guard the same way
  // as null, but would serialise differently and read as a missing field rather
  // than an explicit absence.
  assert.equal(approval.requestMessage, null);
});

test("token is carried through verbatim for the d tag", () => {
  const approval = fromRawApproval(rawApproval());
  // The relay resolves the gate by hex(SHA256(token)) in the `d` tag. Any
  // re-hashing or case change here breaks every grant.
  assert.equal(approval.token, "b".repeat(64));
  assert.match(approval.token, /^[0-9a-f]{64}$/);
});

test("approver spec survives as the exact pubkey", () => {
  const approval = fromRawApproval(rawApproval());
  // check_approver_spec accepts only "any" or 64-hex; a mangled value here
  // would present a gate that no grant can satisfy.
  assert.match(approval.approverSpec, /^[0-9a-f]{64}$/);
});
