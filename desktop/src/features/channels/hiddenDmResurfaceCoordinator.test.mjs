import assert from "node:assert/strict";
import test from "node:test";

import { createHiddenDmResurfaceCoordinator } from "./hiddenDmResurfaceCoordinator.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("a follower arriving mid-attempt retries from the latest event after a failure", async () => {
  const seen = [];
  const gates = [deferred(), deferred()];
  const errors = [];
  const coordinator = createHiddenDmResurfaceCoordinator({
    resurface: async (event) => {
      const index = seen.length;
      seen.push(event.id);
      await gates[index].promise;
    },
    isCurrent: () => true,
    onError: (channelId, error) => errors.push([channelId, error]),
  });

  coordinator.handle("dm-1", { id: "event-a" });
  await Promise.resolve();
  // Follower B lands while attempt A is still in flight.
  coordinator.handle("dm-1", { id: "event-b" });
  assert.deepEqual(seen, ["event-a"]);

  // A fails; the retry re-runs from the latest event (B), which succeeds.
  gates[0].reject(new Error("boom"));
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(seen, ["event-a", "event-b"]);

  gates[1].resolve();
  await gates[1].promise;
  await Promise.resolve();

  assert.equal(errors.length, 1);
  assert.equal(errors[0][0], "dm-1");
});

test("a stale generation's attempt does not delete the live generation's entry", async () => {
  // Model two generations: each real subscription generation creates its own
  // coordinator, so a torn-down generation's cleanup touches only its own map.
  let staleCurrent = true;
  const staleGate = deferred();
  const staleSeen = [];
  const stale = createHiddenDmResurfaceCoordinator({
    resurface: async (event) => {
      staleSeen.push(event.id);
      await staleGate.promise;
    },
    isCurrent: () => staleCurrent,
  });

  const liveSeen = [];
  const live = createHiddenDmResurfaceCoordinator({
    resurface: async (event) => {
      liveSeen.push(event.id);
    },
    isCurrent: () => true,
  });

  // Attempt A begins on the stale generation and suspends.
  stale.handle("dm-1", { id: "event-a" });
  await Promise.resolve();
  assert.deepEqual(staleSeen, ["event-a"]);

  // Generation flips; event B is delivered to the live coordinator and reopens.
  staleCurrent = false;
  live.handle("dm-1", { id: "event-b" });
  await Promise.resolve();
  assert.deepEqual(liveSeen, ["event-b"]);

  // Stale A now retires. Its cleanup cannot touch the live coordinator's map,
  // so a subsequent live follower still starts a fresh attempt.
  staleGate.resolve();
  await staleGate.promise;
  await Promise.resolve();

  live.handle("dm-1", { id: "event-c" });
  await Promise.resolve();
  assert.deepEqual(liveSeen, ["event-b", "event-c"]);
});
