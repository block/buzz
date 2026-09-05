import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient } from "@tanstack/react-query";

import {
  agentConfigSurfaceQueryKey,
  relayAgentsQueryKey,
} from "@/features/agents/hooks";
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

test("local agent changes invalidate every cached config surface, not the relay directory", async () => {
  const client = new QueryClient();
  const first = agentConfigSurfaceQueryKey("agent-a");
  const second = agentConfigSurfaceQueryKey("agent-b");
  for (const key of [first, second, relayAgentsQueryKey]) {
    client.setQueryData(key, { source: "old config" });
  }
  for (const queryKey of LOCAL_AGENT_DATA_QUERY_KEYS) {
    await client.invalidateQueries({ queryKey });
  }
  assert.equal(client.getQueryState(first)?.isInvalidated, true);
  assert.equal(client.getQueryState(second)?.isInvalidated, true);
  assert.equal(client.getQueryState(relayAgentsQueryKey)?.isInvalidated, false);
  client.clear();
});
