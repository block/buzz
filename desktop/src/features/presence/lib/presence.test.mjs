import assert from "node:assert/strict";
import test from "node:test";

import { mergePresenceUpdate, presenceQueryWantsPubkey } from "./presence.ts";

const WILL = "8e39cba681211b3782d0e4483e9343719b9b7be66515252da5491f26421896b1";
const OTHER =
  "44b8e82baaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

test("merge adds an absent pubkey going online (the core bug)", () => {
  const old = {};
  const next = mergePresenceUpdate(old, WILL, "online");
  assert.deepEqual(next, { [WILL]: "online" });
});

test("merge updates an existing pubkey", () => {
  const next = mergePresenceUpdate({ [WILL]: "offline" }, WILL, "online");
  assert.deepEqual(next, { [WILL]: "online" });
});

test("merge returns same reference when status is unchanged", () => {
  const old = { [WILL]: "online" };
  assert.equal(mergePresenceUpdate(old, WILL, "online"), old);
});

test("merge leaves other pubkeys untouched", () => {
  const next = mergePresenceUpdate({ [OTHER]: "away" }, WILL, "online");
  assert.deepEqual(next, { [OTHER]: "away", [WILL]: "online" });
});

test("merge is a no-op on an undefined cache", () => {
  assert.equal(mergePresenceUpdate(undefined, WILL, "online"), undefined);
});

test("query wants a pubkey it requested", () => {
  assert.equal(presenceQueryWantsPubkey(["presence", WILL, OTHER], WILL), true);
});

test("query does not want a pubkey it did not request", () => {
  assert.equal(presenceQueryWantsPubkey(["presence", OTHER], WILL), false);
});

test("bare presence key (no pubkeys) wants nothing", () => {
  assert.equal(presenceQueryWantsPubkey(["presence"], WILL), false);
});
