import assert from "node:assert/strict";
import test from "node:test";

import { createPlaybackSpeedPersistence } from "./playbackSpeedPersistence.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

function harness() {
  const calls = [];
  const saves = [];
  const persist = (speed) => {
    calls.push(speed);
    const save = deferred();
    saves.push(save);
    return save.promise;
  };
  return {
    calls,
    controller: createPlaybackSpeedPersistence(persist),
    saves,
  };
}

function observe(controller) {
  const errors = [];
  const speeds = [];
  const unsubscribe = controller.subscribe({
    setError: (error) => errors.push(error),
    setSpeed: (speed) => speeds.push(speed),
  });
  return { errors, speeds, unsubscribe };
}

test("rapid intents persist serially and finish at the latest speed", async () => {
  const h = harness();
  const listener = observe(h.controller);
  h.controller.commit(1.25);
  h.controller.commit(1.5);
  assert.deepEqual(h.calls, [1.25]);

  h.saves[0].resolve();
  await Promise.resolve();
  assert.deepEqual(h.calls, [1.25, 1.5]);
  h.saves[1].resolve();
  await Promise.resolve();

  assert.equal(listener.speeds.at(-1), 1.5);
});

test("a stale failure does not roll back a newer successful intent", async () => {
  const h = harness();
  const listener = observe(h.controller);
  h.controller.commit(1.25);
  h.controller.commit(1.5);

  h.saves[0].reject(new Error("first failed"));
  await Promise.resolve();
  assert.deepEqual(h.calls, [1.25, 1.5]);
  h.saves[1].resolve();
  await Promise.resolve();

  assert.equal(listener.speeds.at(-1), 1.5);
  assert.equal(listener.errors.at(-1), null);
});

test("the latest failure rolls back to the last confirmed speed", async () => {
  const h = harness();
  const listener = observe(h.controller);
  h.controller.commit(1.25);
  h.saves[0].reject(new Error("save failed"));
  await Promise.resolve();
  await Promise.resolve();

  assert.equal(listener.speeds.at(-1), 1);
  assert.equal(listener.errors.at(-1), "Error: save failed");
});

test("a delayed initial load cannot overwrite a local intent", async () => {
  const h = harness();
  const listener = observe(h.controller);
  h.controller.commit(1.5);
  h.controller.applyLoaded(1);

  assert.equal(listener.speeds.at(-1), 1.5);
  h.saves[0].resolve();
  await Promise.resolve();
});

test("the latest intent survives unmount and is visible after remount", async () => {
  const h = harness();
  const firstMount = observe(h.controller);
  h.controller.commit(1.25);
  h.controller.commit(1.5);
  firstMount.unsubscribe();

  h.saves[0].resolve();
  await Promise.resolve();
  assert.deepEqual(h.calls, [1.25, 1.5]);
  h.saves[1].resolve();
  await Promise.resolve();

  const secondMount = observe(h.controller);
  assert.equal(secondMount.speeds.at(-1), 1.5);
  assert.equal(secondMount.errors.at(-1), null);
});
