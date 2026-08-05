import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAgentFallbacks,
  selectVisibleExternalRelayAgents,
} from "./externalRelayAgents.ts";

const CURRENT_PUBKEY = "a".repeat(64);
const LOCAL_PUBKEY = "b".repeat(64);

function relayAgent({
  pubkey,
  name,
  status = "offline",
  channelIds = ["general"],
  respondTo = "allowlist",
  respondToAllowlist = [CURRENT_PUBKEY],
}) {
  return {
    pubkey,
    name,
    agentType: "coding",
    channels: ["poc-delivery"],
    channelIds,
    capabilities: ["messages"],
    status,
    respondTo,
    respondToAllowlist,
  };
}

test("selectVisibleExternalRelayAgents filters local and inaccessible agents, deduplicates, and sorts by status then name", () => {
  const techPubkey = "1".repeat(64);
  const result = selectVisibleExternalRelayAgents({
    currentPubkey: CURRENT_PUBKEY,
    managedAgentPubkeys: [LOCAL_PUBKEY.toUpperCase()],
    sharedChannelIds: new Set(["general"]),
    relayAgents: [
      relayAgent({
        pubkey: "5".repeat(64),
        name: "QA",
        status: "offline",
      }),
      relayAgent({
        pubkey: techPubkey,
        name: "Tech",
        status: "online",
      }),
      relayAgent({
        pubkey: "3".repeat(64),
        name: "Coding",
        status: "away",
      }),
      relayAgent({
        pubkey: "4".repeat(64),
        name: "CR",
        status: "online",
      }),
      relayAgent({
        pubkey: "2".repeat(64),
        name: "Product",
        status: "online",
      }),
      relayAgent({
        pubkey: techPubkey.toUpperCase(),
        name: "Tech duplicate",
        status: "offline",
      }),
      relayAgent({
        pubkey: LOCAL_PUBKEY,
        name: "Local managed agent",
        status: "online",
      }),
      relayAgent({
        pubkey: "6".repeat(64),
        name: "Unshared agent",
        status: "online",
        channelIds: ["other"],
        respondTo: "anyone",
        respondToAllowlist: [],
      }),
      relayAgent({
        pubkey: "7".repeat(64),
        name: "Owner-only agent",
        status: "online",
        respondTo: "owner-only",
        respondToAllowlist: [],
      }),
    ],
  });

  assert.deepEqual(
    result.map((agent) => agent.name),
    ["CR", "Product", "Tech", "Coding", "QA"],
  );
});

test("selectVisibleExternalRelayAgents keeps an explicitly allowlisted agent outside shared channels", () => {
  const result = selectVisibleExternalRelayAgents({
    currentPubkey: CURRENT_PUBKEY.toUpperCase(),
    managedAgentPubkeys: [],
    sharedChannelIds: new Set(["general"]),
    relayAgents: [
      relayAgent({
        pubkey: "8".repeat(64),
        name: "Allowlisted",
        channelIds: ["other"],
        respondToAllowlist: [CURRENT_PUBKEY],
      }),
    ],
  });

  assert.deepEqual(
    result.map((agent) => agent.name),
    ["Allowlisted"],
  );
});

test("selectVisibleExternalRelayAgents falls back to shared channel bots missing from the Relay agent directory", () => {
  const fallbackPubkey = "9".repeat(64);
  const explicitlyRestrictedPubkey = "c".repeat(64);
  const result = selectVisibleExternalRelayAgents({
    currentPubkey: CURRENT_PUBKEY,
    managedAgentPubkeys: [],
    sharedChannelIds: new Set(["general"]),
    relayAgents: [
      relayAgent({
        pubkey: explicitlyRestrictedPubkey,
        name: "Restricted directory agent",
        respondTo: "owner-only",
        respondToAllowlist: [],
      }),
    ],
    channelAgents: [
      relayAgent({
        pubkey: fallbackPubkey,
        name: "Channel-only agent",
        respondTo: null,
        respondToAllowlist: [],
      }),
      relayAgent({
        pubkey: explicitlyRestrictedPubkey,
        name: "Restricted channel member",
        respondTo: null,
        respondToAllowlist: [],
      }),
    ],
  });

  assert.deepEqual(
    result.map((agent) => agent.name),
    ["Channel-only agent"],
  );
});

test("buildChannelAgentFallbacks aggregates bot memberships and live presence", () => {
  const botPubkey = "d".repeat(64);
  const result = buildChannelAgentFallbacks({
    channels: [
      { id: "general", name: "General" },
      { id: "delivery", name: "Delivery" },
    ],
    membersByChannelId: {
      general: [
        {
          pubkey: botPubkey,
          displayName: "Product Agent",
          role: "bot",
          isAgent: true,
        },
        {
          pubkey: "e".repeat(64),
          displayName: "Human",
          role: "member",
          isAgent: false,
        },
        {
          pubkey: "f".repeat(64),
          displayName: null,
          role: "bot",
          isAgent: true,
        },
      ],
      delivery: [
        {
          pubkey: botPubkey.toUpperCase(),
          displayName: "Product Agent",
          role: "bot",
          isAgent: true,
        },
      ],
    },
    presence: { [botPubkey]: "online" },
  });

  assert.deepEqual(result, [
    {
      pubkey: botPubkey,
      name: "Product Agent",
      agentType: "external",
      channels: ["General", "Delivery"],
      channelIds: ["general", "delivery"],
      capabilities: [],
      status: "online",
      respondTo: null,
      respondToAllowlist: [],
    },
  ]);
});

test("buildChannelAgentFallbacks treats a registered member as an agent only in explicitly registered channels", () => {
  const registeredPubkey = "a1".repeat(32);
  const result = buildChannelAgentFallbacks({
    channels: [
      { id: "allowed", name: "Allowed" },
      { id: "other", name: "Other" },
    ],
    membersByChannelId: {
      allowed: [
        {
          pubkey: registeredPubkey.toUpperCase(),
          displayName: "Stale Relay Name",
          role: "member",
          isAgent: false,
        },
      ],
      other: [
        {
          pubkey: registeredPubkey,
          displayName: "Stale Relay Name",
          role: "member",
          isAgent: false,
        },
      ],
    },
    presence: { [registeredPubkey]: "online" },
    registrations: [
      {
        id: "research",
        name: "Research Agent",
        pubkey: registeredPubkey,
        channelIds: ["allowed"],
        state: "active",
        enabled: true,
      },
    ],
  });

  assert.deepEqual(result, [
    {
      pubkey: registeredPubkey,
      name: "Research Agent",
      agentType: "external",
      channels: ["Allowed"],
      channelIds: ["allowed"],
      capabilities: [],
      status: "online",
      respondTo: null,
      respondToAllowlist: [],
      registryState: "active",
    },
  ]);
});

test("buildChannelAgentFallbacks keeps unregistered members as people and forces failed registrations offline", () => {
  const failedPubkey = "b1".repeat(32);
  const humanPubkey = "c1".repeat(32);
  const disabledPubkey = "d1".repeat(32);
  const provisioningPubkey = "e1".repeat(32);
  const result = buildChannelAgentFallbacks({
    channels: [{ id: "allowed", name: "Allowed" }],
    membersByChannelId: {
      allowed: [
        {
          pubkey: failedPubkey,
          displayName: "Old failed name",
          role: "member",
          isAgent: false,
        },
        {
          pubkey: humanPubkey,
          displayName: "Human",
          role: "member",
          isAgent: false,
        },
        {
          pubkey: disabledPubkey,
          displayName: "Disabled",
          role: "member",
          isAgent: false,
        },
        {
          pubkey: provisioningPubkey,
          displayName: "Provisioning",
          role: "member",
          isAgent: false,
        },
      ],
    },
    presence: {
      [failedPubkey]: "online",
      [humanPubkey]: "online",
      [disabledPubkey]: "online",
      [provisioningPubkey]: "online",
    },
    registrations: [
      {
        id: "failed",
        name: "Failed Agent",
        pubkey: failedPubkey,
        channelIds: ["allowed"],
        state: "failed",
        enabled: true,
      },
      {
        id: "disabled",
        name: "Disabled Agent",
        pubkey: disabledPubkey,
        channelIds: ["allowed"],
        state: "active",
        enabled: false,
      },
      {
        id: "provisioning",
        name: "Provisioning Agent",
        pubkey: provisioningPubkey,
        channelIds: ["allowed"],
        state: "provisioning",
        enabled: true,
      },
    ],
  });

  assert.deepEqual(
    result.map(({ name, pubkey, registryState, status }) => ({
      name,
      pubkey,
      registryState,
      status,
    })),
    [
      {
        name: "Failed Agent",
        pubkey: failedPubkey,
        registryState: "failed",
        status: "offline",
      },
    ],
  );
});
