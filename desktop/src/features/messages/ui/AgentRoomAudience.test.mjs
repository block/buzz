import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_ROOM_ROUNDTABLE_PROMPT,
  getReadyAgentRoomAgents,
  getUnaddressedAgentRoomAgents,
} from "./AgentRoomAudience.tsx";

test("agent room audience deduplicates and excludes addressed agents", () => {
  const agents = [
    { pubkey: "AA", name: "Research", status: "running" },
    { pubkey: "bb", name: "Finance", status: "deployed" },
    { pubkey: "aa", name: "Duplicate", status: "stopped" },
  ];

  assert.deepEqual(
    getUnaddressedAgentRoomAgents(agents, [" BB "]).map((agent) => agent.name),
    ["Research"],
  );

  assert.match(AGENT_ROOM_ROUNDTABLE_PROMPT, /evidence/i);
  assert.match(AGENT_ROOM_ROUNDTABLE_PROMPT, /assumptions/i);
});

test("agent room actions exclude stopped agents", () => {
  assert.deepEqual(
    getReadyAgentRoomAgents([
      { pubkey: "aa", name: "Ready", status: "running" },
      { pubkey: "bb", name: "Remote", status: "deployed" },
      { pubkey: "cc", name: "Stopped", status: "stopped" },
    ]).map((agent) => agent.name),
    ["Ready", "Remote"],
  );
});
