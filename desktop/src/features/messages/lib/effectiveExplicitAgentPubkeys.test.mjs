import assert from "node:assert/strict";
import test from "node:test";

import { filterEffectiveExplicitAgentPubkeys } from "./effectiveExplicitAgentPubkeys.ts";

const agentA = "a".repeat(64);
const agentB = "b".repeat(64);
const person = "c".repeat(64);

test("send without inviting excludes removed agents from audience promotion", () => {
  assert.deepEqual(
    filterEffectiveExplicitAgentPubkeys([agentA, agentB], [agentA, person]),
    [agentA],
  );
});

test("effective audience promotion keeps authored order and dedupes", () => {
  assert.deepEqual(
    filterEffectiveExplicitAgentPubkeys(
      [agentB.toUpperCase(), agentA, agentB],
      [agentA, agentB],
    ),
    [agentB, agentA],
  );
});
