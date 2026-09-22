import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { combineObserverIngestionAgents } from "./useAgentObserverIngestion.ts";

const ME = "aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234";
const OTHER =
  "bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321";
const AGENT_LOCAL =
  "cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111";
const AGENT_REMOTE =
  "dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222";
const AGENT_FOREIGN =
  "eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333";
const AGENT_COMMUNITY =
  "ffff4444ffff4444ffff4444ffff4444ffff4444ffff4444ffff4444ffff4444";

describe("combineObserverIngestionAgents", () => {
  it("keeps managed agents with their real status", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "running" }],
      [],
      new Map(),
      ME,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "running" }]);
  });

  it("adds declared-owned relay agents as deployed", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE],
      new Map([[AGENT_REMOTE, ME]]),
      ME,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_REMOTE, status: "deployed" }]);
  });

  it("excludes relay agents owned by someone else", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_FOREIGN],
      new Map([[AGENT_FOREIGN, OTHER]]),
      ME,
    );
    assert.deepEqual(result, []);
  });

  it("excludes relay agents with no declared owner", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE],
      new Map(),
      ME,
    );
    assert.deepEqual(result, []);
  });

  it("does not duplicate an agent that is both managed and on the relay", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "stopped" }],
      [AGENT_LOCAL],
      new Map([[AGENT_LOCAL, ME]]),
      ME,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "stopped" }]);
  });

  it("matches ownership case-insensitively", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE.toUpperCase()],
      new Map([[AGENT_REMOTE, ME.toUpperCase()]]),
      ME,
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_REMOTE.toUpperCase(), status: "deployed" },
    ]);
  });

  it("returns only managed agents when identity is not resolved yet", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "running" }],
      [AGENT_REMOTE],
      new Map([[AGENT_REMOTE, ME]]),
      undefined,
      [AGENT_COMMUNITY],
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "running" }]);
  });

  it("adds catalog community bots as deployed", () => {
    const result = combineObserverIngestionAgents(
      [],
      [],
      new Map(),
      ME,
      [AGENT_COMMUNITY],
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_COMMUNITY, status: "deployed" },
    ]);
  });

  it("does not duplicate a community bot that is already managed", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_COMMUNITY, status: "stopped" }],
      [],
      new Map(),
      ME,
      [AGENT_COMMUNITY],
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_COMMUNITY, status: "stopped" },
    ]);
  });

  it("does not duplicate a community bot that is already declared-owned", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_COMMUNITY],
      new Map([[AGENT_COMMUNITY, ME]]),
      ME,
      [AGENT_COMMUNITY.toUpperCase()],
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_COMMUNITY, status: "deployed" },
    ]);
  });

  it("folds community bots alongside managed and owned agents", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "running" }],
      [AGENT_REMOTE],
      new Map([[AGENT_REMOTE, ME]]),
      ME,
      [AGENT_COMMUNITY],
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_LOCAL, status: "running" },
      { pubkey: AGENT_REMOTE, status: "deployed" },
      { pubkey: AGENT_COMMUNITY, status: "deployed" },
    ]);
  });

  it("skips empty community bot pubkeys", () => {
    const result = combineObserverIngestionAgents([], [], new Map(), ME, [
      "",
      "   ",
    ]);
    assert.deepEqual(result, []);
  });

  it("keeps reserved community bot pubkeys trusted without a catalog", () => {
    // Mirrors mention hard-routing: Activity must ingest Stitch/etc even when
    // useCommunityBotsQuery returns [].
    const STITCH =
      "54d8ee67ae6bb50255b851ac53a5b4d235497e0c8ea6a43d5b05850389c531f7";
    const result = combineObserverIngestionAgents([], [], new Map(), ME, [
      STITCH,
    ]);
    assert.deepEqual(result, [{ pubkey: STITCH, status: "deployed" }]);
  });
});
