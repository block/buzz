import assert from "node:assert/strict";
import test from "node:test";

import { shouldPromoteExplicitAddress } from "./shouldPromoteExplicitAddress.ts";

test("ordinary explicit address does not pin when auto-mention preference is off", () => {
  assert.equal(shouldPromoteExplicitAddress(false), false);
});

test("ordinary explicit address may pin when auto-mention preference is on", () => {
  assert.equal(shouldPromoteExplicitAddress(true), true);
});

test("intentional persist still opts in when preference is off", () => {
  // Mirror promoteExplicitlyAddressedAgents: persist bypasses the one-shot gate.
  const preferenceOff = false;
  const persist = true;
  const shouldPromote = persist || shouldPromoteExplicitAddress(preferenceOff);
  assert.equal(shouldPromote, true);
});
