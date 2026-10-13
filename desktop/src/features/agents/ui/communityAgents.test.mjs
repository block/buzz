import assert from "node:assert/strict";
import test from "node:test";

import {
  communityAgentChannelLabels,
  communityAgentChannelSummary,
  selectCommunityAgents,
} from "./communityAgents.ts";

function agent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    ownerPubkey: null,
    name: "Research Lead",
    agentType: "agent",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "unknown",
    respondTo: null,
    respondToAllowlist: [],
    ...overrides,
  };
}

test("resolves authoritative channel ids to visible names", () => {
  const remote = agent({
    channels: [],
    channelIds: ["research-id", "portfolio-id"],
  });

  assert.deepEqual(
    communityAgentChannelLabels(remote, {
      "research-id": "research-desk",
      "portfolio-id": "portfolio-desk",
    }),
    ["research-desk", "portfolio-desk"],
  );
  assert.equal(
    communityAgentChannelSummary(remote, {
      "research-id": "research-desk",
      "portfolio-id": "portfolio-desk",
    }),
    "#research-desk · #portfolio-desk",
  );
});

test("uses current visible names instead of stale directory names", () => {
  const remote = agent({
    channels: ["old-alpha", "old-onchain"],
    channelIds: ["alpha-id", "onchain-id"],
  });

  assert.equal(
    communityAgentChannelSummary(remote, {
      "alpha-id": "alpha-desk",
      "onchain-id": "onchain-desk",
    }),
    "#alpha-desk · #onchain-desk",
  );
});

test("does not expose unresolved directory names when ids are available", () => {
  const remote = agent({
    channels: ["private-name", "stale-name", "extra-name"],
    channelIds: ["one", "two"],
  });

  assert.deepEqual(communityAgentChannelLabels(remote, {}), []);
  assert.equal(communityAgentChannelSummary(remote, {}), "2 shared channels");
});

test("uses directory names only for legacy records without channel ids", () => {
  assert.equal(
    communityAgentChannelSummary(
      agent({ channels: ["alpha-desk", "onchain-desk", "watchlist"] }),
      {},
    ),
    "#alpha-desk · #onchain-desk · +1",
  );
  assert.equal(communityAgentChannelSummary(agent(), {}), "No shared channels");
});

test("excludes locally managed pubkeys case-insensitively", () => {
  const local = agent({ pubkey: "AB".repeat(32), name: "Local" });
  const remote = agent({ pubkey: "cd".repeat(32), name: "Remote" });

  assert.deepEqual(
    selectCommunityAgents(
      [remote, local],
      (pubkey) => pubkey.toLowerCase() === "ab".repeat(32),
    ).map((candidate) => candidate.pubkey),
    [remote.pubkey],
  );
});
