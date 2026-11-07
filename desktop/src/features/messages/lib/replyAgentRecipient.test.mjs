import assert from "node:assert/strict";
import { test } from "node:test";

import { replyAgentRecipient } from "./replyAgentRecipient.mjs";

const agent = "a".repeat(64);
const owner = "b".repeat(64);
const person = "c".repeat(64);
const agentMessage = {
  id: "1".repeat(64),
  pubkey: agent,
  tags: [["auth", owner, "", "sig"]],
};
const personMessage = { id: "2".repeat(64), pubkey: person, tags: [] };

test("a reply to an agent's message tags the agent", () => {
  assert.equal(
    replyAgentRecipient(
      agentMessage.id,
      [[personMessage], [agentMessage]],
      owner,
    ),
    agent,
  );
});

test("a reply to a person's message adds no tag", () => {
  assert.equal(
    replyAgentRecipient(
      personMessage.id,
      [[personMessage, agentMessage]],
      owner,
    ),
    null,
  );
});

test("an agent replying to its own message, or an unknown parent, adds no tag", () => {
  assert.equal(
    replyAgentRecipient(agentMessage.id, [[agentMessage]], agent),
    null,
  );
  assert.equal(
    replyAgentRecipient("3".repeat(64), [[agentMessage]], owner),
    null,
  );
});
