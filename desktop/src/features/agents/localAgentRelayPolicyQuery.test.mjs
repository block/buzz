import assert from "node:assert/strict";
import test from "node:test";

import { localAgentRelayAllowedQueryKey } from "./localAgentRelayPolicyQuery.ts";

test("local agent relay policy cache is scoped by community", () => {
  assert.notDeepEqual(
    localAgentRelayAllowedQueryKey("community-a"),
    localAgentRelayAllowedQueryKey("community-b"),
  );
  assert.deepEqual(localAgentRelayAllowedQueryKey(null), [
    "local-agent-relay-allowed",
    "none",
  ]);
});
