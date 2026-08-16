import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  account,
  allResolved,
  bindDisposition,
  classifyRequest,
  COMPLETE_COVERAGE,
  deriveObligation,
  isMarkedRequest,
  REQUEST_KINDS,
} from "./disposition.ts";

// The same corpora the Rust verifier runs (crates/buzz-core/tests/
// nip_ad_conformance.rs).
//
// - nip-ad-conformance.json is hand-written and pins the two IMPLEMENTATIONS
//   to each other, which is how the earlier silent divergence was closed.
// - nip-ad-lifecycle-exhaustive.json is generated with expectations computed
//   from declarative rules rather than from either implementation, so it also
//   pins both to the SPEC — the gap that let a hand-written transition table
//   contradict working code with every test still green.
const corpusPath = (name) =>
  fileURLToPath(new URL(`../../../../docs/nips/${name}`, import.meta.url));

const CORPUS = JSON.parse(
  readFileSync(corpusPath("nip-ad-conformance.json"), "utf8"),
);
const EXHAUSTIVE = JSON.parse(
  readFileSync(corpusPath("nip-ad-lifecycle-exhaustive.json"), "utf8"),
);

const KIND_AGENT_DISPOSITION = 44300;

const CONSTANTS = (() => {
  const map = new Map();
  for (const [key, value] of Object.entries(CORPUS.constants)) {
    map.set(`$${key}`, value);
  }
  // Derived so the corpus can express the uppercase-target case without
  // duplicating a 64-char literal.
  map.set("$AGENT_UPPER", CORPUS.constants.agent.toUpperCase());
  return map;
})();

function resolve(raw) {
  if (CONSTANTS.has(raw)) {
    return CONSTANTS.get(raw);
  }
  let out = raw;
  for (const [placeholder, value] of CONSTANTS) {
    out = out.split(placeholder).join(value);
  }
  return out;
}

function resolveTags(rows) {
  return rows.map((row) => row.map(resolve));
}

// Events are built from the CORPUS's kind, never this implementation's.
// Defaulting to REQUEST_KINDS[0] on both sides is what let a Rust/TS
// divergence on the request-kind set pass both suites.
const CORPUS_REQUEST_KIND = CORPUS.requestKinds[0];

function requestEvent(tags, kind = CORPUS_REQUEST_KIND) {
  return {
    id: CORPUS.constants.requestId,
    pubkey: CORPUS.constants.requester,
    kind,
    created_at: 100,
    content: "@agent do the thing",
    tags,
  };
}

/** The canonical valid request every binding and lifecycle case derives from. */
function canonicalObligation() {
  const classified = classifyRequest(
    requestEvent(resolveTags(CORPUS.requestClassification[0].tags)),
  );
  assert.equal(
    classified.kind,
    "valid",
    "corpus case 0 must be a valid request",
  );
  return classified.obligation;
}

function dispositionEvent(id, createdAt, state) {
  return {
    id,
    pubkey: CORPUS.constants.agent,
    kind: KIND_AGENT_DISPOSITION,
    created_at: createdAt,
    content: JSON.stringify({ disposition: state, reason: `r-${state}` }),
    tags: [
      ["e", CORPUS.constants.requestId],
      ["h", CORPUS.constants.channel],
      ["p", CORPUS.constants.requester],
      ["disposition", state],
    ],
  };
}

function assertDerived(name, derived, expect) {
  assert.deepEqual(derived.outcome, expect.outcome, `${name}: wrong outcome`);
  assert.equal(
    derived.latestObservation,
    expect.latestObservation,
    `${name}: wrong latest observation`,
  );
  assert.deepEqual(
    derived.warnings,
    expect.warnings,
    `${name}: wrong warnings`,
  );
  assert.equal(
    derived.outcome.kind === "settled",
    expect.resolved,
    `${name}: wrong resolved`,
  );
}

test("request kinds match the corpus", () => {
  assert.deepEqual(
    [...REQUEST_KINDS],
    CORPUS.requestKinds,
    "this implementation's request-kind set disagrees with the corpus",
  );
});

test("request classification matches the corpus", () => {
  for (const testCase of CORPUS.requestClassification) {
    const { name, expect } = testCase;
    const event = requestEvent(
      resolveTags(testCase.tags),
      testCase.kind ?? CORPUS_REQUEST_KIND,
    );
    const classified = classifyRequest(event);
    assert.equal(classified.kind, expect.kind, `${name}: wrong class`);

    if (expect.kind === "valid") {
      // Pin the whole obligation, not just the target: a classifier that got
      // the channel or requester wrong would still pass a target-only check.
      assert.equal(
        classified.obligation.targetAgentPubkey,
        resolve(expect.targetAgent),
        `${name}: wrong target`,
      );
      assert.equal(classified.obligation.requestId, CORPUS.constants.requestId);
      assert.equal(
        classified.obligation.requesterPubkey,
        CORPUS.constants.requester,
      );
      assert.equal(classified.obligation.channelId, CORPUS.constants.channel);
    } else if (expect.kind === "not_request") {
      assert.equal(
        isMarkedRequest(event),
        false,
        `${name}: marker disagreement`,
      );
    } else {
      assert.equal(classified.reason, expect.reason, `${name}: wrong reason`);
    }
  }
});

test("binding matches the corpus", () => {
  const obligation = canonicalObligation();
  for (const testCase of CORPUS.binding) {
    const { name, expect } = testCase;
    const result = bindDisposition(obligation, {
      id: "d0",
      pubkey: resolve(testCase.signer),
      kind: testCase.kind ?? KIND_AGENT_DISPOSITION,
      created_at: 200,
      content: resolve(testCase.content),
      tags: resolveTags(testCase.tags),
    });
    assert.equal(result.bound, expect.bound, `${name}: wrong bound`);
    if (expect.bound) {
      assert.equal(result.state, expect.state, `${name}: wrong state`);
    } else {
      assert.equal(result.reason, expect.reason, `${name}: wrong bind failure`);
    }
  }
});

test("lifecycle matches the corpus", () => {
  const obligation = canonicalObligation();
  for (const testCase of CORPUS.lifecycle) {
    const events = testCase.events.map(([id, createdAt, state]) =>
      dispositionEvent(id, createdAt, state),
    );
    assertDerived(
      testCase.name,
      deriveObligation(obligation, events),
      testCase.expect,
    );
  }
});

test("exhaustive lifecycle matches the generated corpus", () => {
  const obligation = canonicalObligation();
  assert.equal(
    EXHAUSTIVE.cases.length,
    EXHAUSTIVE.caseCount,
    "corpus caseCount disagrees with its own case list",
  );
  for (const { history, expect } of EXHAUSTIVE.cases) {
    // Index order is sort order: increasing created_at, distinct ids.
    const events = history.map((state, i) =>
      dispositionEvent(`d${String(i).padStart(4, "0")}`, 100 + i, state),
    );
    assertDerived(
      `[${history.join(" -> ")}]`,
      deriveObligation(obligation, events),
      expect,
    );
  }
});

test("accounting never reports a malformed request as an agent gap", () => {
  const cases = CORPUS.requestClassification
    .map((c, i) => ({ ...c, index: i }))
    .filter(
      (c) =>
        (c.expect.kind === "invalid" || c.expect.kind === "unsupported") &&
        // The unsupported-kind case is about the event's kind, not its tags,
        // and is covered by the classification test.
        c.kind == null,
    );

  const requests = cases.map((c) => ({
    ...requestEvent(resolveTags(c.tags)),
    id: String(c.index).padStart(64, "0"),
  }));

  const acc = account(requests, [], COMPLETE_COVERAGE);
  assert.equal(
    acc.invalidRequests.length,
    cases.filter((c) => c.expect.kind === "invalid").length,
  );
  assert.equal(
    acc.unsupportedRequests.length,
    cases.filter((c) => c.expect.kind === "unsupported").length,
  );
  assert.deepEqual(
    acc.unanswered,
    [],
    "malformed requests must never be counted as unanswered obligations",
  );
  assert.equal(
    allResolved(acc),
    false,
    "unanswerable requests block a clean claim",
  );
});

test("an unbound claim is excluded from state but reported", () => {
  // Spoof attempts must not vanish silently — an auditor needs to see them,
  // they just must not affect the outcome.
  const spoof = {
    ...dispositionEvent("d1", 200, "completed"),
    pubkey: CORPUS.constants.human,
  };
  const acc = account(
    [requestEvent(resolveTags(CORPUS.requestClassification[0].tags))],
    [spoof],
    COMPLETE_COVERAGE,
  );
  assert.deepEqual(acc.unanswered, [CORPUS.constants.requestId]);
  assert.equal(acc.rejectedClaims.length, 1);
  assert.equal(acc.rejectedClaims[0].failure, "not_target_agent");
  assert.equal(acc.rejectedClaims[0].signer, CORPUS.constants.human);
});
