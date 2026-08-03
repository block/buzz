import assert from "node:assert/strict";
import test from "node:test";

import { connectedAgentMembershipAdded } from "./connectedAgentChannelIntent.ts";

const AGENT =
  "4687f50de3a9e235e28eb58d68b0746062d7be6401bbf78a766bbd6f96ffe3c9";

test("reports the connected agent's successful membership write", () => {
  assert.equal(
    connectedAgentMembershipAdded(AGENT, {
      added: [AGENT.toUpperCase()],
      errors: [],
    }),
    true,
  );
});

test("does not treat another batch entry as this agent's success", () => {
  assert.equal(
    connectedAgentMembershipAdded(AGENT, {
      added: ["f".repeat(64)],
      errors: [],
    }),
    false,
  );
});

test("surfaces the matching relay membership error", () => {
  assert.throws(
    () =>
      connectedAgentMembershipAdded(AGENT, {
        added: [],
        errors: [{ pubkey: AGENT, error: "channel is archived" }],
      }),
    /channel is archived/,
  );
});

test("ignores an error for a different batch entry", () => {
  assert.equal(
    connectedAgentMembershipAdded(AGENT, {
      added: [AGENT],
      errors: [{ pubkey: "f".repeat(64), error: "not this agent" }],
    }),
    true,
  );
});
