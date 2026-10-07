import assert from "node:assert/strict";
import test from "node:test";

import { mergeMentionRecipients } from "./useMentionSendFlow.helpers.ts";

test("address-locked agents join explicit mentions without duplicating recipients", () => {
  const explicit = ["A".repeat(64), "b".repeat(64)];
  const locked = ["a".repeat(64), "C".repeat(64)];

  assert.deepEqual(mergeMentionRecipients(explicit, locked), [
    "a".repeat(64),
    "b".repeat(64),
    "c".repeat(64),
  ]);
});
