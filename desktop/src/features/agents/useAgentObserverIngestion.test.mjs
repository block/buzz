import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { combineObserverIngestionAgents } from "./useAgentObserverIngestion.ts";

const AGENT_LOCAL =
  "cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111";
const AGENT_REMOTE =
  "dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222";
const AGENT_CHANNEL =
  "eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333";

describe("combineObserverIngestionAgents", () => {
  it("keeps managed agents with their real status", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "running" }],
      [],
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "running" }]);
  });

  it("registers every channel-visible relay agent for shared ingestion", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE, AGENT_CHANNEL],
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_REMOTE, status: "deployed" },
      { pubkey: AGENT_CHANNEL, status: "deployed" },
    ]);
  });

  it("does not duplicate an agent present locally and on the relay", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "stopped" }],
      [AGENT_LOCAL.toUpperCase()],
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "stopped" }]);
  });

  it("deduplicates relay discovery case-insensitively", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE, AGENT_REMOTE.toUpperCase()],
    );
    assert.deepEqual(result, [{ pubkey: AGENT_REMOTE, status: "deployed" }]);
  });
});
