import assert from "node:assert/strict";
import test from "node:test";

import { selectOwnedExternalAgents } from "./useOwnedExternalAgents.ts";

const OWNER = "a".repeat(64);
const EXTERNAL = "b".repeat(64);
const LOCAL = "c".repeat(64);
const OTHER = "d".repeat(64);

function relayAgent(pubkey, name) {
  return { pubkey, name };
}

test("selects owner-attested relay agents that are not locally managed", () => {
  const selected = selectOwnedExternalAgents({
    currentPubkey: OWNER.toUpperCase(),
    managedAgents: [{ pubkey: LOCAL }],
    relayAgents: [
      relayAgent(EXTERNAL, "relay fallback"),
      relayAgent(LOCAL, "local duplicate"),
      relayAgent(OTHER, "other owner's agent"),
    ],
    profiles: {
      [EXTERNAL]: {
        avatarUrl: "https://example.com/external.png",
        displayName: "External Pi",
        ownerPubkey: OWNER,
      },
      [LOCAL]: {
        avatarUrl: null,
        displayName: "Local Pi",
        ownerPubkey: OWNER,
      },
      [OTHER]: {
        avatarUrl: null,
        displayName: "Other Pi",
        ownerPubkey: "e".repeat(64),
      },
    },
  });

  assert.deepEqual(selected, [
    {
      avatarUrl: "https://example.com/external.png",
      name: "External Pi",
      pubkey: EXTERNAL,
    },
  ]);
});

test("falls back to the relay name and deduplicates normalized pubkeys", () => {
  const selected = selectOwnedExternalAgents({
    currentPubkey: OWNER,
    managedAgents: [],
    relayAgents: [
      relayAgent(EXTERNAL.toUpperCase(), "Relay Pi"),
      relayAgent(EXTERNAL, "Duplicate"),
    ],
    profiles: {
      [EXTERNAL]: {
        avatarUrl: null,
        displayName: "   ",
        ownerPubkey: OWNER,
      },
    },
  });

  assert.deepEqual(selected, [
    { avatarUrl: null, name: "Relay Pi", pubkey: EXTERNAL },
  ]);
});
