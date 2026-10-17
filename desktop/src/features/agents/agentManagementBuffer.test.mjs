import assert from "node:assert/strict";
import test from "node:test";

import {
  advanceAgentManagementRequest,
  classifyAgentManagementOrigin,
  enqueueAgentManagementRequest,
} from "./agentManagementBuffer.ts";
import { AGENT_MANAGEMENT_REQUEST } from "./agentManagement.ts";

const AGENT = "a".repeat(64);
const CHANNEL = "channel-1";
const OWNED_AGENT = [{ pubkey: AGENT }];
const SHARED_CHANNEL = [
  { id: CHANNEL, isMember: true, memberPubkeys: [AGENT] },
];

function queuedRequest(requestId) {
  return {
    agentPubkey: AGENT,
    request: {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "update",
      requestId,
      request: {
        channelId: CHANNEL,
        agentName: "Agent",
        displayName: requestId,
      },
    },
  };
}

test("queues agent management requests while another draft is active", () => {
  const first = queuedRequest("first");
  const second = queuedRequest("second");
  const third = queuedRequest("third");

  let state = { active: null, queued: [] };
  state = enqueueAgentManagementRequest(state, first);
  state = enqueueAgentManagementRequest(state, second);
  state = enqueueAgentManagementRequest(state, third);

  assert.equal(state.active, first);
  assert.deepEqual(state.queued, [second, third]);
});

test("advances queued agent management requests in arrival order", () => {
  const first = queuedRequest("first");
  const second = queuedRequest("second");
  const third = queuedRequest("third");
  let state = {
    active: first,
    queued: [second, third],
  };

  state = advanceAgentManagementRequest(state);
  assert.equal(state.active, second);
  assert.deepEqual(state.queued, [third]);

  state = advanceAgentManagementRequest(state);
  assert.equal(state.active, third);
  assert.deepEqual(state.queued, []);

  state = advanceAgentManagementRequest(state);
  assert.equal(state.active, null);
  assert.deepEqual(state.queued, []);
});

test("buffers a draft until ownership and channel data resolve", () => {
  assert.equal(
    classifyAgentManagementOrigin(undefined, SHARED_CHANNEL, AGENT, CHANNEL),
    "buffer",
  );
  assert.equal(
    classifyAgentManagementOrigin(OWNED_AGENT, undefined, AGENT, CHANNEL),
    "buffer",
  );
});

test("accepts an owned agent drafting from a shared channel", () => {
  assert.equal(
    classifyAgentManagementOrigin(OWNED_AGENT, SHARED_CHANNEL, AGENT, CHANNEL),
    "accept",
  );
});

test("rejects a draft when the owner or agent is outside the claimed channel", () => {
  assert.equal(
    classifyAgentManagementOrigin(
      OWNED_AGENT,
      [{ id: CHANNEL, isMember: false, memberPubkeys: [AGENT] }],
      AGENT,
      CHANNEL,
    ),
    "reject",
  );
  assert.equal(
    classifyAgentManagementOrigin(
      OWNED_AGENT,
      [{ id: CHANNEL, isMember: true, memberPubkeys: [] }],
      AGENT,
      CHANNEL,
    ),
    "reject",
  );
});

test("rejects a draft from an agent this Desktop does not own", () => {
  assert.equal(
    classifyAgentManagementOrigin(
      [{ pubkey: "b".repeat(64) }],
      SHARED_CHANNEL,
      AGENT,
      CHANNEL,
    ),
    "reject",
  );
});
