import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  getOwnedRelayWorkingAgents,
  mergeWorkingAgents,
  resolveActiveWorkingChannelNames,
} from "./useActiveWorkingChannelsById.ts";

const VIEWER_PUBKEY =
  "80c5f18be5aafa62cf6198c6335963ba3306b595288117c8ea2f805fc9bdc94a";
const OWNED_RELAY_AGENT_PUBKEY =
  "a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00";

describe("resolveActiveWorkingChannelNames", () => {
  it("resolves active agent pubkeys to working agent names", () => {
    const resolved = resolveActiveWorkingChannelNames(
      {
        channelId: "chan-1",
        anchorAt: 0,
        agentCount: 2,
        agentPubkeys: ["AAAA", "bbbb"],
      },
      [
        { pubkey: "aaaa", name: "Ned" },
        { pubkey: "BBBB", name: "Bart" },
      ],
    );

    assert.deepEqual(resolved.agentNames, ["Ned", "Bart"]);
  });

  it("omits unresolved active agents from the resolved names", () => {
    const resolved = resolveActiveWorkingChannelNames(
      {
        channelId: "chan-1",
        anchorAt: 0,
        agentCount: 2,
        agentPubkeys: ["AAAA", "cccc"],
      },
      [{ pubkey: "aaaa", name: "Ned" }],
    );

    assert.deepEqual(resolved.agentNames, ["Ned"]);
  });

  it("resolves owned relay agent names", () => {
    const resolved = resolveActiveWorkingChannelNames(
      {
        channelId: "chan-1",
        anchorAt: 0,
        agentCount: 1,
        agentPubkeys: [OWNED_RELAY_AGENT_PUBKEY.toUpperCase()],
      },
      [{ pubkey: OWNED_RELAY_AGENT_PUBKEY, name: "nadia" }],
    );

    assert.deepEqual(resolved.agentNames, ["nadia"]);
  });
});

describe("getOwnedRelayWorkingAgents", () => {
  it("keeps relay agents whose NIP-OA owner is the current viewer", () => {
    assert.deepEqual(
      getOwnedRelayWorkingAgents(
        [
          { pubkey: OWNED_RELAY_AGENT_PUBKEY, name: "nadia" },
          { pubkey: "other-agent", name: "nelson" },
        ],
        {
          [OWNED_RELAY_AGENT_PUBKEY]: {
            displayName: "nadia",
            avatarUrl: null,
            nip05Handle: null,
            ownerPubkey: VIEWER_PUBKEY.toUpperCase(),
            isAgent: true,
          },
          "other-agent": {
            displayName: "nelson",
            avatarUrl: null,
            nip05Handle: null,
            ownerPubkey: "someone-else",
            isAgent: true,
          },
        },
        VIEWER_PUBKEY,
      ),
      [{ pubkey: OWNED_RELAY_AGENT_PUBKEY, name: "nadia", status: "deployed" }],
    );
  });

  it("returns no relay agents without a current viewer", () => {
    assert.deepEqual(
      getOwnedRelayWorkingAgents(
        [{ pubkey: OWNED_RELAY_AGENT_PUBKEY, name: "nadia" }],
        {},
        undefined,
      ),
      [],
    );
  });

  it("drops relay agents with missing profiles or null owners", () => {
    assert.deepEqual(
      getOwnedRelayWorkingAgents(
        [
          { pubkey: OWNED_RELAY_AGENT_PUBKEY, name: "nadia" },
          { pubkey: "ownerless-agent", name: "ralph" },
        ],
        {
          "ownerless-agent": {
            displayName: "ralph",
            avatarUrl: null,
            nip05Handle: null,
            ownerPubkey: null,
            isAgent: true,
          },
        },
        VIEWER_PUBKEY,
      ),
      [],
    );
  });
});

describe("mergeWorkingAgents", () => {
  it("dedupes owned relay agents behind locally managed agents", () => {
    assert.deepEqual(
      mergeWorkingAgents(
        [{ pubkey: "AAAA", name: "Ned", status: "running" }],
        [
          { pubkey: "aaaa", name: "Relay Ned", status: "deployed" },
          {
            pubkey: OWNED_RELAY_AGENT_PUBKEY,
            name: "nadia",
            status: "deployed",
          },
        ],
      ),
      [
        { pubkey: "AAAA", name: "Ned", status: "running" },
        { pubkey: OWNED_RELAY_AGENT_PUBKEY, name: "nadia", status: "deployed" },
      ],
    );
  });
});
