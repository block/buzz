import assert from "node:assert/strict";
import test from "node:test";

import { buildAgentRespondToUpdate } from "./agentRespondToUpdate.ts";

const ALLOWED_PUBKEY = "c".repeat(64);

test("instance edits always submit the form's respond-to mode", () => {
  assert.deepEqual(buildAgentRespondToUpdate("owner-only", []), {
    respondTo: "owner-only",
    respondToAllowlist: undefined,
  });
});

test("allowlist edits always submit the complete form allowlist", () => {
  const allowlist = [ALLOWED_PUBKEY];

  const update = buildAgentRespondToUpdate("allowlist", allowlist);

  assert.deepEqual(update, {
    respondTo: "allowlist",
    respondToAllowlist: [ALLOWED_PUBKEY],
  });
  assert.notEqual(update.respondToAllowlist, allowlist);
});

test("non-allowlist edits do not overwrite a preserved allowlist", () => {
  assert.deepEqual(buildAgentRespondToUpdate("anyone", [ALLOWED_PUBKEY]), {
    respondTo: "anyone",
    respondToAllowlist: undefined,
  });
});
