import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_ADDRESS_MENTION_MARKER,
  buildAgentAddressMentionTags,
  getAgentAddressMentionPubkeys,
} from "./agentAddressMention.mjs";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

test("builds address metadata only for recipients that survived admission", () => {
  assert.deepEqual(
    buildAgentAddressMentionTags([ALICE.toUpperCase(), BOB], [ALICE]),
    [["mention", ALICE, AGENT_ADDRESS_MENTION_MARKER]],
  );
});

test("reads ordered address metadata without treating ordinary mentions as tray state", () => {
  assert.deepEqual(
    getAgentAddressMentionPubkeys([
      ["p", BOB],
      ["mention", BOB],
      ["mention", ALICE.toUpperCase(), AGENT_ADDRESS_MENTION_MARKER],
      ["mention", ALICE, AGENT_ADDRESS_MENTION_MARKER],
    ]),
    [ALICE],
  );
});
