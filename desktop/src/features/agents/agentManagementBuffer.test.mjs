import assert from "node:assert/strict";
import test from "node:test";

import { classifyAgentManagementOrigin } from "./agentManagementBuffer.ts";

const AGENT = "a".repeat(64);
const CHANNEL = "channel-1";
const OWNED_AGENT = [{ pubkey: AGENT }];
const SHARED_CHANNEL = [
  { id: CHANNEL, isMember: true, memberPubkeys: [AGENT] },
];

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
