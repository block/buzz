import assert from "node:assert/strict";
import test from "node:test";

import { relayAgentsQueryKey } from "@/features/agents/hooks";
import { LOCAL_AGENT_DATA_QUERY_KEYS } from "./useAgentsDataRefresh.ts";

const serializedLocalKeys = LOCAL_AGENT_DATA_QUERY_KEYS.map((key) =>
  JSON.stringify(key),
);

test("local agent refresh never invalidates the relay directory", () => {
  assert.equal(
    serializedLocalKeys.includes(JSON.stringify(relayAgentsQueryKey)),
    false,
    "local reconciliation must not trigger a relay-wide directory rebuild",
  );
});
