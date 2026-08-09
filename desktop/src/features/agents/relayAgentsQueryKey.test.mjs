import assert from "node:assert/strict";
import test from "node:test";

import { relayAgentsQueryKeyForScope } from "./relayAgentsQueryScope.ts";

test("relay agent cache keys differ for relay URLs", () => {
  const first = relayAgentsQueryKeyForScope(
    "wss://first.example.test",
    "community-a",
    "a".repeat(64),
  );
  const second = relayAgentsQueryKeyForScope(
    "wss://second.example.test",
    "community-b",
    "a".repeat(64),
  );

  assert.notDeepEqual(first, second);
  assert.equal(first[1], "wss://first.example.test");
  assert.equal(second[1], "wss://second.example.test");
  assert.equal(
    relayAgentsQueryKeyForScope(" WSS://FIRST.EXAMPLE.TEST/// ")[1],
    "wss://first.example.test",
  );
});
