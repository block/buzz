import assert from "node:assert/strict";
import test from "node:test";

import { pickProfileAgent } from "./pickProfileAgent.ts";

test("the shared profile target prefers the active persona instance", () => {
  const stopped = {
    name: "Earlier instance",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const running = {
    name: "Current instance",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(pickProfileAgent([stopped, running]), running);
  assert.equal(pickProfileAgent([running, stopped]), running);
});
