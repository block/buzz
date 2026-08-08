import assert from "node:assert/strict";
import test from "node:test";

import { resolveTrayActivities, resolveTrayAgentName } from "./useTrayMenu.ts";

const REMOTE_AGENT_PUBKEY = "1".repeat(64);

test("resolveTrayAgentName uses a hydrated remote-agent profile", () => {
  assert.equal(
    resolveTrayAgentName({
      knownAgentName: undefined,
      profile: {
        avatarUrl: null,
        displayName: "Hermes",
        isAgent: true,
        nip05Handle: null,
        ownerPubkey: "2".repeat(64),
      },
      pubkey: REMOTE_AGENT_PUBKEY,
    }),
    "Hermes",
  );
});

test("resolveTrayActivities replaces a completed activity fallback after profile hydration", () => {
  const activities = resolveTrayActivities({
    activities: [
      {
        activityId: `recent:channel:${REMOTE_AGENT_PUBKEY}:1`,
        agentName: "Agent 111111…111111",
        agentPubkey: REMOTE_AGENT_PUBKEY,
        channelId: "channel",
        channelName: "hermes-acceptance",
        elapsed: "1s",
      },
    ],
    knownAgentNames: new Map(),
    profiles: {
      [REMOTE_AGENT_PUBKEY]: {
        avatarUrl: null,
        displayName: "Hermes",
        isAgent: true,
        nip05Handle: null,
        ownerPubkey: "2".repeat(64),
      },
    },
  });

  assert.equal(activities[0].agentName, "Hermes");
  assert.equal("agentPubkey" in activities[0], false);
});
