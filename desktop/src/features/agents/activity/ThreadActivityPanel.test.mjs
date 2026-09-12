import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  agentsWithThreadActivity,
  threadScopeIsLive,
  turnFilterForThreadAgents,
} from "./ThreadActivityPanel.tsx";

describe("threadScopeIsLive", () => {
  it("requires a normalized pubkey to be both open in the thread and working in the channel", () => {
    assert.equal(threadScopeIsLive(new Set(["Agent-A"]), []), false);
    assert.equal(threadScopeIsLive(new Set(), ["agent-a"]), false);
    assert.equal(threadScopeIsLive(new Set(["agent-a"]), ["agent-b"]), false);
    assert.equal(threadScopeIsLive(new Set([" AGENT-A "]), ["agent-a"]), true);
    assert.equal(threadScopeIsLive(new Set(["agent-a"]), [" AGENT-A "]), true);
  });
});

describe("agentsWithThreadActivity", () => {
  it("keeps only agents with turn activity in the thread, matching pubkeys case-insensitively", () => {
    const agents = [
      { pubkey: "Agent-A", name: "Has activity" },
      { pubkey: "agent-b", name: "Other thread only" },
      { pubkey: "agent-c", name: "Empty bucket" },
    ];
    const turnIdsByAgent = new Map([
      ["agent-a", new Set(["turn-1"])],
      ["agent-c", new Set()],
    ]);

    assert.deepEqual(agentsWithThreadActivity(agents, turnIdsByAgent), [
      agents[0],
    ]);
  });
});

describe("turnFilterForThreadAgents", () => {
  it("keys the thread turn filter by input agent pubkey casing", () => {
    const turnIds = new Set(["turn-1"]);
    const filter = turnFilterForThreadAgents(
      [{ pubkey: "Agent-A", name: "Has activity" }],
      new Map([["agent-a", turnIds]]),
    );

    assert.equal(filter.get("Agent-A"), turnIds);
    assert.equal(filter.has("agent-a"), false);
  });
});
