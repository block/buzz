import assert from "node:assert/strict";
import { test } from "node:test";

import { createAvatarProfileSync } from "./avatarProfileSync.ts";

const INPUT = {
  avatarUrl: "https://old-relay.example/avatar.png",
  relayUrl: "wss://old-relay.example",
  expectedPubkey: "pubkey",
  expectedAvatarUrl: null,
};

function createHarness({ initialState = "pending", saveProfile } = {}) {
  let presentation = { displayUrl: INPUT.avatarUrl, state: initialState };
  let listener = () => {};
  let unsubscribeCount = 0;
  const saves = [];
  const refreshed = [];
  const sync = createAvatarProfileSync({
    getPresentation: () => presentation,
    subscribe: (nextListener) => {
      listener = nextListener;
      return () => {
        unsubscribeCount += 1;
      };
    },
    saveProfile:
      saveProfile ??
      (async (input) => {
        saves.push(input);
        return { avatarUrl: input.avatarUrl, pubkey: input.expectedPubkey };
      }),
    refreshCaches: async (profile, input) => {
      refreshed.push({ profile, input });
    },
  });

  return {
    get unsubscribeCount() {
      return unsubscribeCount;
    },
    listener: () => listener(),
    refreshed,
    saves,
    setState: (state) => {
      presentation = { ...presentation, state };
    },
    sync,
  };
}

async function flushPromises() {
  await new Promise((resolve) => setImmediate(resolve));
}

test("saves at the captured relay and refreshes caches after verification", async () => {
  const harness = createHarness();
  harness.sync.saveWhenReady(INPUT);

  harness.setState("ready");
  harness.listener();
  await flushPromises();

  assert.deepEqual(harness.saves, [INPUT]);
  assert.equal(harness.refreshed.length, 1);
  assert.equal(harness.refreshed[0].input.relayUrl, INPUT.relayUrl);
  assert.equal(harness.unsubscribeCount, 1);
});

test("community reset cancels a pending avatar save", async () => {
  const harness = createHarness();
  harness.sync.saveWhenReady(INPUT);

  harness.sync.reset();
  harness.setState("ready");
  harness.listener();
  await flushPromises();

  assert.deepEqual(harness.saves, []);
  assert.equal(harness.unsubscribeCount, 1);
});

test("a reset sync accepts deferred work from the next community", async () => {
  const harness = createHarness();
  harness.sync.reset();
  harness.setState("ready");
  const nextInput = {
    ...INPUT,
    relayUrl: "wss://next-relay.example",
  };

  harness.sync.saveWhenReady(nextInput);
  await flushPromises();

  assert.deepEqual(harness.saves, [nextInput]);
  assert.equal(harness.refreshed.length, 1);
});

test("cache refresh follows only a successful save", async () => {
  let rejectSave;
  const savePromise = new Promise((_, reject) => {
    rejectSave = reject;
  });
  const harness = createHarness({
    initialState: "ready",
    saveProfile: () => savePromise,
  });
  harness.sync.saveWhenReady(INPUT);

  rejectSave(new Error("stale baseline"));
  await flushPromises();

  assert.deepEqual(harness.refreshed, []);
  assert.equal(harness.unsubscribeCount, 1);
});
