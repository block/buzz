import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildChannelAgentSessionCandidates,
  getChannelAgentSessionAgents,
} from "./useChannelAgentSessions.ts";

const CHANNEL_ID = "7fdd5a94-7523-5b86-b574-9124de6cdc0d";
const OTHER_CHANNEL_ID = "adafd400-706c-5c13-bf02-bc2ebb45b1c7";
const MEDIA_AGENT =
  "e38c4fec9cddd9833d05a9dd7279606dcf321b95c3739c265423e211e8f1b789";
const MANAGED_AGENT =
  "db2f4298db2f4298db2f4298db2f4298db2f4298db2f4298db2f4298db2f4298";

const CHANNEL = { id: CHANNEL_ID, name: "general" };

function mediaSession(overrides = {}) {
  return { agentPubkey: MEDIA_AGENT, channelId: CHANNEL_ID, ...overrides };
}

function pubkeySet(...pubkeys) {
  return new Set(pubkeys.map((pubkey) => pubkey.toLowerCase()));
}

describe("media-only agents qualify for a session panel", () => {
  // The regression this whole path exists for: an external agent that
  // publishes video emits no kind:24200 observer frame and no kind:20002
  // typing indicator, so every activity-derived signal is silent. It is also
  // not desktop-managed and has no kind:10100 registration, and nothing marks
  // an external agent as a bot at add-time — so its roster role is `member`.
  // Before the media source it matched none of the three candidate rules, the
  // panel could not be opened, AgentMediaSurface never mounted, and no viewer
  // token was ever requested.
  const memberRoster = [
    { pubkey: MEDIA_AGENT, role: "member", displayName: "Cara" },
  ];

  it("does not qualify a plain member without a session", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: memberRoster,
      managedAgents: [],
      relayAgents: [],
    });

    assert.equal(candidates.length, 0);
  });

  it("qualifies the agent on the announcement alone", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: memberRoster,
      managedAgents: [],
      mediaSessions: [mediaSession()],
      relayAgents: [],
    });

    assert.equal(candidates.length, 1);
    assert.equal(candidates[0].pubkey, MEDIA_AGENT);
    assert.equal(candidates[0].agentSource, "media");
    // Nothing can interrupt a turn that no ACP harness is running.
    assert.equal(candidates[0].canInterruptTurn, false);
    // The session is the scope, so the channel is declared on the candidate.
    assert.deepEqual(candidates[0].channelIds, [CHANNEL_ID]);
  });

  it("keeps the agent through the channel filter", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: memberRoster,
      managedAgents: [],
      mediaSessions: [mediaSession()],
      relayAgents: [],
    });

    const agents = getChannelAgentSessionAgents({
      activeChannel: CHANNEL,
      activeChannelId: CHANNEL_ID,
      agents: candidates,
      channelMembers: memberRoster,
      mediaSessionPubkeys: pubkeySet(MEDIA_AGENT),
    });

    assert.deepEqual(
      agents.map((agent) => agent.pubkey),
      [MEDIA_AGENT],
    );
  });

  it("keeps the agent even when it is not on the roster at all", () => {
    // Membership and a live session are separate claims. An agent that
    // announces a session in a channel is visible in it for as long as the
    // session lives, whatever the roster snapshot says.
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: [],
      managedAgents: [],
      mediaSessions: [mediaSession()],
      relayAgents: [],
    });

    const agents = getChannelAgentSessionAgents({
      activeChannel: CHANNEL,
      activeChannelId: CHANNEL_ID,
      agents: candidates,
      channelMembers: [],
      mediaSessionPubkeys: pubkeySet(MEDIA_AGENT),
    });

    assert.deepEqual(
      agents.map((agent) => agent.pubkey),
      [MEDIA_AGENT],
    );
  });

  it("names the agent from its pubkey when no agent record exists", () => {
    const [candidate] = buildChannelAgentSessionCandidates({
      channelMembers: [],
      managedAgents: [],
      mediaSessions: [mediaSession()],
      relayAgents: [],
    });

    // resolveUserLabel prefers the kind:0 profile at render; this fallback
    // shows only for an agent that has published no profile.
    assert.equal(candidate.name, "e38c4fec…b789");
  });

  it("matches the announcing pubkey case-insensitively", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: [{ pubkey: MEDIA_AGENT, role: "bot" }],
      managedAgents: [],
      mediaSessions: [mediaSession({ agentPubkey: MEDIA_AGENT.toUpperCase() })],
      relayAgents: [],
    });

    // One agent, not two: the roster bot already claimed the key.
    assert.equal(candidates.length, 1);
    assert.equal(candidates[0].agentSource, "member-bot");
  });
});

describe("a media session does not overwrite a richer agent record", () => {
  it("keeps the managed record and adds the session's channel", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: [],
      managedAgents: [
        { pubkey: MANAGED_AGENT, name: "OpenCode", status: "deployed" },
      ],
      mediaSessions: [mediaSession({ agentPubkey: MANAGED_AGENT })],
      relayAgents: [
        {
          pubkey: MANAGED_AGENT,
          name: "OpenCode",
          status: "online",
          channelIds: [OTHER_CHANNEL_ID],
          channels: [],
        },
      ],
    });

    assert.equal(candidates.length, 1);
    const [agent] = candidates;
    assert.equal(agent.name, "OpenCode");
    assert.equal(agent.agentSource, "managed");
    // Still interruptible — the media session says nothing about its harness.
    assert.equal(agent.canInterruptTurn, true);
    assert.deepEqual(agent.channelIds, [OTHER_CHANNEL_ID, CHANNEL_ID]);
  });

  it("does not duplicate a channel the record already declares", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: [],
      managedAgents: [],
      mediaSessions: [mediaSession({ agentPubkey: MANAGED_AGENT })],
      relayAgents: [
        {
          pubkey: MANAGED_AGENT,
          name: "OpenCode",
          status: "online",
          channelIds: [CHANNEL_ID],
          channels: [],
        },
      ],
    });

    assert.deepEqual(candidates[0].channelIds, [CHANNEL_ID]);
  });
});

describe("a session scopes only its own channel", () => {
  it("does not qualify the agent in a channel it is not live in", () => {
    const candidates = buildChannelAgentSessionCandidates({
      channelMembers: [],
      managedAgents: [],
      mediaSessions: [mediaSession({ channelId: OTHER_CHANNEL_ID })],
      relayAgents: [],
    });

    const agents = getChannelAgentSessionAgents({
      activeChannel: CHANNEL,
      activeChannelId: CHANNEL_ID,
      agents: candidates,
      channelMembers: [],
      // The hook derives this set from the active channel's subscription, so
      // a session in another channel contributes nothing here.
      mediaSessionPubkeys: new Set(),
    });

    assert.deepEqual(agents, []);
  });
});
