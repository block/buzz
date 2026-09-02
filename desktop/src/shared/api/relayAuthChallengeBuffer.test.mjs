import assert from "node:assert/strict";
import test from "node:test";

import { RelayAuthChallengeBuffer } from "./relayAuthChallengeBuffer.ts";

test("returns an eager AUTH challenge once the matching generation is ready", () => {
  const buffer = new RelayAuthChallengeBuffer();

  buffer.store("challenge-a", 7);

  assert.equal(buffer.take(7), "challenge-a");
  assert.equal(buffer.take(7), null);
});

test("drops an AUTH challenge from a stale connection generation", () => {
  const buffer = new RelayAuthChallengeBuffer();

  buffer.store("stale", 3);

  assert.equal(buffer.take(4), null);
  assert.equal(buffer.take(3), null);
});

test("keeps only the latest eager AUTH challenge", () => {
  const buffer = new RelayAuthChallengeBuffer();

  buffer.store("first", 5);
  buffer.store("second", 5);

  assert.equal(buffer.take(5), "second");
});
