import assert from "node:assert/strict";
import test from "node:test";

import { presenceQueryKeyForScope } from "./hooks.ts";

test("presence cache keys differ for relay URLs", () => {
  const pubkey = "a".repeat(64);
  const first = presenceQueryKeyForScope("wss://first.example.test", [pubkey]);
  const second = presenceQueryKeyForScope("wss://second.example.test", [
    pubkey,
  ]);

  assert.notDeepEqual(first, second);
  assert.equal(first[1], "wss://first.example.test");
  assert.equal(second[1], "wss://second.example.test");
  assert.deepEqual(
    presenceQueryKeyForScope(" WSS://FIRST.EXAMPLE.TEST/// ", [pubkey]),
    first,
  );
});
