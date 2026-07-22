import assert from "node:assert/strict";
import { test } from "node:test";

import { createAvatarProfileSync } from "./avatarProfileSync.ts";

const AVATAR_URL = "https://old-relay.example/avatar.png";
const PUBKEY = "pubkey";

function createHarness({ initialState = "pending", getProfile } = {}) {
  let presentation = { displayUrl: AVATAR_URL, state: initialState };
  let listener = () => {};
  let unsubscribeCount = 0;
  const updates = [];
  const sync = createAvatarProfileSync({
    getPresentation: () => presentation,
    getProfile:
      getProfile ?? (async () => ({ avatarUrl: null, pubkey: PUBKEY })),
    subscribe: (nextListener) => {
      listener = nextListener;
      return () => {
        unsubscribeCount += 1;
      };
    },
    updateProfile: async (input) => {
      updates.push(input);
    },
  });

  return {
    get unsubscribeCount() {
      return unsubscribeCount;
    },
    listener: () => listener(),
    setState: (state) => {
      presentation = { ...presentation, state };
    },
    sync,
    updates,
  };
}

async function flushPromises() {
  await new Promise((resolve) => setImmediate(resolve));
}

test("saves an avatar after verification succeeds", async () => {
  const harness = createHarness();
  harness.sync.saveWhenReady(AVATAR_URL, PUBKEY, null);

  harness.setState("ready");
  harness.listener();
  await flushPromises();

  assert.deepEqual(harness.updates, [{ avatarUrl: AVATAR_URL }]);
  assert.equal(harness.unsubscribeCount, 1);
});

test("community reset cancels a pending avatar save", async () => {
  const harness = createHarness();
  harness.sync.saveWhenReady(AVATAR_URL, PUBKEY, null);

  harness.sync.reset();
  harness.setState("ready");
  harness.listener();
  await flushPromises();

  assert.deepEqual(harness.updates, []);
  assert.equal(harness.unsubscribeCount, 1);
});

test("community reset invalidates a profile read already in flight", async () => {
  let resolveProfile;
  const profilePromise = new Promise((resolve) => {
    resolveProfile = resolve;
  });
  const harness = createHarness({
    getProfile: () => profilePromise,
    initialState: "ready",
  });
  harness.sync.saveWhenReady(AVATAR_URL, PUBKEY, null);

  harness.sync.reset();
  resolveProfile({ avatarUrl: null, pubkey: PUBKEY });
  await flushPromises();

  assert.deepEqual(harness.updates, []);
  assert.equal(harness.unsubscribeCount, 1);
});
