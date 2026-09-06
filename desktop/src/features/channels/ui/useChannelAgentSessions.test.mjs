import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAgentSessionCandidates,
  getChannelAgentSessionAgents,
} from "./useChannelAgentSessions.ts";

const OWNER =
  "dd57c78422bccf568feb2a7ae5bcf4d7ebefc2c6c54bf56a26faeb9e0b08d36b";
const CLAUDE_HOST =
  "8df15208c09bccecce4d77cbf73874fbaf1441f4a7747925e5e62e422d9f0a1b";
const CLAUDE_DEV =
  "302583f7ba1fff34e7d8814779bfe1cdc6c2382bda33a62a9266dc34fc959bf5";

function dmChannel(id, participantPubkeys) {
  return {
    id,
    name: `dm-${id}`,
    channelType: "dm",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [OWNER, ...participantPubkeys],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

function streamChannel(id, name) {
  return { ...dmChannel(id, [OWNER]), channelType: "stream", name };
}

function relayAgent(pubkey, channelIds, channels = []) {
  return {
    pubkey,
    name: "relay-agent",
    status: "deployed",
    agentSource: "relay",
    canInterruptTurn: false,
    channelIds,
    channels,
  };
}

test("BUZZ-DESKTOP-003: relay agent is included inside its own DM despite missing declared channel scope", () => {
  const dm = dmChannel("dm-uuid-1", [OWNER, CLAUDE_HOST]);
  const agents = buildChannelAgentSessionCandidates({
    channelMembers: [],
    managedAgents: [],
    relayAgents: [
      { pubkey: CLAUDE_HOST, name: "Claude-Host", status: "online" },
    ],
  });
  const result = getChannelAgentSessionAgents({
    activeChannel: dm,
    activeChannelId: dm.id,
    agents,
    channelMembers: [],
  });
  assert.deepEqual(
    result.map((agent) => agent.pubkey),
    [CLAUDE_HOST],
  );
});

test("BUZZ-DESKTOP-003: DM participant match is case-insensitive", () => {
  const dm = dmChannel("dm-uuid-2", [OWNER, CLAUDE_HOST.toUpperCase()]);
  const agents = buildChannelAgentSessionCandidates({
    channelMembers: [],
    managedAgents: [],
    relayAgents: [
      { pubkey: CLAUDE_HOST, name: "Claude-Host", status: "online" },
    ],
  });
  const result = getChannelAgentSessionAgents({
    activeChannel: dm,
    activeChannelId: dm.id,
    agents,
    channelMembers: [],
  });
  assert.equal(result.length, 1);
});

test("BUZZ-DESKTOP-003: relay agent that is not a DM participant stays excluded", () => {
  const dm = dmChannel("dm-uuid-3", [OWNER, CLAUDE_HOST]);
  const agents = buildChannelAgentSessionCandidates({
    channelMembers: [],
    managedAgents: [],
    relayAgents: [{ pubkey: CLAUDE_DEV, name: "Claude-DEV", status: "online" }],
  });
  const result = getChannelAgentSessionAgents({
    activeChannel: dm,
    activeChannelId: dm.id,
    agents,
    channelMembers: [],
  });
  assert.deepEqual(result, []);
});

test("BUZZ-DESKTOP-003: declared channel_ids path still works for normal channels", () => {
  const stream = streamChannel(
    "bb814c05-53bc-4b30-b7ef-8efe2f5f985c",
    "patterndy-development",
  );
  const agents = [
    relayAgent(CLAUDE_DEV, [stream.id]),
    relayAgent(CLAUDE_HOST, ["4558ff4c-d01d-4361-a8d1-9783d1c75f82"]),
  ];
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents,
    channelMembers: undefined,
  });
  assert.deepEqual(
    result.map((agent) => agent.pubkey),
    [CLAUDE_DEV],
  );
});

test("MyBuzz: current bot membership admits a relay agent without declared channel scope", () => {
  const stream = streamChannel("dynamic-stream", "dynamic-stream");
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents: [relayAgent(CLAUDE_HOST, [])],
    channelMembers: [{ pubkey: CLAUDE_HOST, role: "bot" }],
  });

  assert.deepEqual(
    result.map((agent) => agent.pubkey),
    [CLAUDE_HOST],
  );
});

test("MyBuzz: current owner membership admits a known relay agent", () => {
  const stream = streamChannel("dynamic-stream", "dynamic-stream");
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents: [relayAgent(CLAUDE_HOST, [])],
    channelMembers: [{ pubkey: CLAUDE_HOST, role: "owner" }],
  });

  assert.deepEqual(
    result.map((agent) => agent.pubkey),
    [CLAUDE_HOST],
  );
});

test("MyBuzz: an ordinary owner is not a relay activity candidate", () => {
  const stream = streamChannel("dynamic-stream", "dynamic-stream");
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents: [],
    channelMembers: [{ pubkey: CLAUDE_HOST, role: "owner" }],
  });

  assert.deepEqual(result, []);
});

test("MyBuzz: a present empty membership snapshot rejects stale declared channel scope", () => {
  const stream = streamChannel("dynamic-stream", "dynamic-stream");
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents: [relayAgent(CLAUDE_HOST, [stream.id])],
    channelMembers: [],
  });

  assert.deepEqual(result, []);
});

test("MyBuzz: admin and member membership do not admit a relay agent to stream activity", () => {
  const stream = streamChannel("dynamic-stream", "dynamic-stream");
  for (const role of ["admin", "member"]) {
    const result = getChannelAgentSessionAgents({
      activeChannel: stream,
      activeChannelId: stream.id,
      agents: [relayAgent(CLAUDE_HOST, [stream.id])],
      channelMembers: [{ pubkey: CLAUDE_HOST, role }],
    });

    assert.deepEqual(result, []);
  }
});

test("MyBuzz: undefined membership snapshot retains relay declared-scope fallback", () => {
  const stream = streamChannel("dynamic-stream", "dynamic-stream");
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents: [relayAgent(CLAUDE_HOST, [stream.id])],
    channelMembers: undefined,
  });

  assert.deepEqual(
    result.map((agent) => agent.pubkey),
    [CLAUDE_HOST],
  );
});

test("MyBuzz: DM eligibility keeps its declared-scope fallback with a membership snapshot", () => {
  const dm = dmChannel("dm-legacy-scope", [OWNER]);
  const result = getChannelAgentSessionAgents({
    activeChannel: dm,
    activeChannelId: dm.id,
    agents: [relayAgent(CLAUDE_HOST, [dm.id])],
    channelMembers: [],
  });

  assert.deepEqual(
    result.map((agent) => agent.pubkey),
    [CLAUDE_HOST],
  );
});

test("BUZZ-DESKTOP-003: participant rule applies only to DM channels", () => {
  // A stream channel whose participantPubkeys happen to contain the agent
  // must NOT admit it via the DM rule — only declared scope (or real
  // membership on the member-bot/managed paths) does.
  const stream = {
    ...streamChannel("stream-uuid-x", "some-channel"),
    participantPubkeys: [OWNER, CLAUDE_HOST],
  };
  const agents = buildChannelAgentSessionCandidates({
    channelMembers: [],
    managedAgents: [],
    relayAgents: [
      { pubkey: CLAUDE_HOST, name: "Claude-Host", status: "online" },
    ],
  });
  const result = getChannelAgentSessionAgents({
    activeChannel: stream,
    activeChannelId: stream.id,
    agents,
    channelMembers: [],
  });
  assert.deepEqual(result, []);
});
