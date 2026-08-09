import assert from "node:assert/strict";
import test from "node:test";

import { teamCatalogQueryKey } from "./teamCatalogHooks.ts";

test("shared catalog cache keys differ for relay URLs", () => {
  const first = teamCatalogQueryKey("wss://first.example.test");
  const second = teamCatalogQueryKey("wss://second.example.test");

  assert.deepEqual(first, ["shared-team-catalog", "wss://first.example.test"]);
  assert.notDeepEqual(first, second);
  assert.deepEqual(teamCatalogQueryKey(" WSS://FIRST.EXAMPLE.TEST/// "), first);
});
