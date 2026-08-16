import assert from "node:assert/strict";
import test from "node:test";

import {
  decideRequestMarking,
  requestAgentPubkeysFor,
} from "./requestMarking.ts";

const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);

test("no agent mentioned is an ordinary message", () => {
  const marking = decideRequestMarking([], false);
  assert.deepEqual(marking, { kind: "none" });
  assert.deepEqual(requestAgentPubkeysFor(marking), []);
});

test("exactly one agent creates a tracked obligation", () => {
  const marking = decideRequestMarking([AGENT], false);
  assert.deepEqual(marking, { kind: "tracked", targetPubkey: AGENT });
  assert.deepEqual(requestAgentPubkeysFor(marking), [AGENT]);
});

test("the same agent mentioned twice is still one target", () => {
  // Two mentions of one agent is one obligation. Emitting the tag twice
  // would be a duplicate target, which the shared verifier rejects as
  // non-canonical — so the composer must never produce that shape.
  const marking = decideRequestMarking([AGENT, AGENT], false);
  assert.deepEqual(marking, { kind: "tracked", targetPubkey: AGENT });
  assert.deepEqual(requestAgentPubkeysFor(marking), [AGENT]);
});

test("two agents are reported as multi-agent, never silently marked", () => {
  // v1 cannot say whether either agent may answer or both must. The
  // composer's job here is to say so, not to quietly send an untracked
  // message and let the sender find out later that nothing was recorded.
  const marking = decideRequestMarking([AGENT, OTHER_AGENT], false);
  assert.deepEqual(marking, {
    kind: "multi-agent",
    targetPubkeys: [AGENT, OTHER_AGENT],
  });
  assert.deepEqual(
    requestAgentPubkeysFor(marking),
    [],
    "a multi-target request would be classified unsupported and could never be discharged",
  );
});

test("opting out suppresses tracking for a single agent", () => {
  const marking = decideRequestMarking([AGENT], true);
  assert.deepEqual(marking, { kind: "opted-out" });
  assert.deepEqual(requestAgentPubkeysFor(marking), []);
});

test("opting out with no agent mentioned is still just an ordinary message", () => {
  // The opt-out is about suppressing an obligation that would otherwise
  // exist; with no agent there is nothing to suppress, and reporting
  // "opted-out" would make the banner claim a choice the sender never faced.
  assert.deepEqual(decideRequestMarking([], true), { kind: "none" });
});

test("the marking decision never yields more than one agent tag", () => {
  // The invariant the whole v1 contract rests on, asserted directly rather
  // than inferred from the cases above.
  for (const mentioned of [
    [],
    [AGENT],
    [AGENT, AGENT],
    [AGENT, OTHER_AGENT],
    [AGENT, OTHER_AGENT, AGENT],
  ]) {
    for (const optedOut of [false, true]) {
      assert.ok(
        requestAgentPubkeysFor(decideRequestMarking(mentioned, optedOut))
          .length <= 1,
        `${mentioned.length} mentions, optedOut=${optedOut}`,
      );
    }
  }
});
