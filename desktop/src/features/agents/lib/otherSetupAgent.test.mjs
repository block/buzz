import assert from "node:assert/strict";
import test from "node:test";

import { isOtherSetupAgent } from "./otherSetupAgent.ts";

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);

test("fails closed while the local managed directory is unresolved", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: false,
      currentPubkey: OWNER,
      managedAgents: [],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    false,
  );
});

test("labels a viewer-owned non-local identity as another setup", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: true,
      currentPubkey: OWNER,
      managedAgents: [],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    true,
  );
});
