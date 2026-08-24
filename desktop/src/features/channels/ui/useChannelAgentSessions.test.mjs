import assert from "node:assert/strict";
import test from "node:test";

import { shouldClearUnresolvedAgentSession } from "./useChannelAgentSessions.ts";

const unresolvedSession = {
  agentSessionAgents: [],
  agentsLoaded: true,
  openAgentSessionPubkey: "agent-pubkey",
  profilePanelPubkey: null,
};

test("dedicated activity windows preserve unresolved sessions for an empty state", () => {
  assert.equal(
    shouldClearUnresolvedAgentSession({
      ...unresolvedSession,
      preserveUnresolvedSession: true,
    }),
    false,
  );
});

test("the in-app panel still clears stale unresolved sessions", () => {
  assert.equal(
    shouldClearUnresolvedAgentSession({
      ...unresolvedSession,
      preserveUnresolvedSession: false,
    }),
    true,
  );
});
