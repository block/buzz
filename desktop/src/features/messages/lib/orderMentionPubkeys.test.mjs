import assert from "node:assert/strict";
import test from "node:test";

import { orderMentionPubkeysByText } from "./orderMentionPubkeys.ts";

const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);

test("orders eligible mention pubkeys by authored text instead of map insertion", () => {
  const ordered = orderMentionPubkeysByText(
    "@Vogue please pair with @Morgarita",
    { morgarita: AGENT_A, vogue: AGENT_B },
    () => true,
  );

  assert.deepEqual(ordered, [AGENT_B, AGENT_A]);
});

test("dedupes aliases at their earliest authored position", () => {
  const ordered = orderMentionPubkeysByText(
    "@Morg please pair with @Vogue and @Morgarita",
    { morgarita: AGENT_A, vogue: AGENT_B, morg: AGENT_A },
    () => true,
  );

  assert.deepEqual(ordered, [AGENT_A, AGENT_B]);
});
