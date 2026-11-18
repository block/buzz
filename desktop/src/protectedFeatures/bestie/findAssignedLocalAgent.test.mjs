import assert from "node:assert/strict";
import test from "node:test";

import { findAssignedLocalAgent } from "./findAssignedLocalAgent.ts";

const LOCAL = {
  backend: { type: "local" },
  pubkey: "a".repeat(64),
};
const REMOTE = {
  backend: { type: "provider" },
  pubkey: "b".repeat(64),
};

test("resolves only an existing local Bestie assignment", () => {
  assert.equal(
    findAssignedLocalAgent([LOCAL, REMOTE], { agentPubkey: "A".repeat(64) }),
    LOCAL,
  );
  assert.equal(
    findAssignedLocalAgent([LOCAL, REMOTE], { agentPubkey: REMOTE.pubkey }),
    null,
  );
  assert.equal(
    findAssignedLocalAgent([LOCAL], { agentPubkey: "c".repeat(64) }),
    null,
  );
  assert.equal(findAssignedLocalAgent([LOCAL], null), null);
});
