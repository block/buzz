import assert from "node:assert/strict";
import { test } from "node:test";

import {
  createAvatarProfileSync,
  createProfileCacheRefreshQueue,
} from "./avatarProfileSync.ts";

const INPUT = {
  avatarUrl: "https://old-relay.example/avatar.png",
  relayUrl: "wss://old-relay.example",
  expectedPubkey: "pubkey",
  expectedAvatarUrl: null,
};

function createHarness({
  initialState = "pending",
  saveProfile,
  getActivePubkey = async () => INPUT.expectedPubkey,
  scheduleRetry,
} = {}) {
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
    getActivePubkey,
    refreshCaches: async (profile, input) => {
      refreshed.push({ profile, input });
    },
    scheduleRetry,
  });

  return {
    get unsubscribeCount() {
      return unsubscribeCount;
    },
    listener: () => listener(),
    refreshed,
    saves,
    removePresentation: () => {
      presentation = null;
    },
    setState: (state) => {
      presentation = {
        ...(presentation ?? { displayUrl: INPUT.avatarUrl }),
        state,
      };
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

test("skips cache refresh when the active identity changes during save", async () => {
  let resolveSave;
  const savePromise = new Promise((resolve) => {
    resolveSave = resolve;
  });
  let activePubkey = INPUT.expectedPubkey;
  const harness = createHarness({
    initialState: "ready",
    saveProfile: () => savePromise,
    getActivePubkey: async () => activePubkey,
  });
  harness.sync.saveWhenReady(INPUT);

  activePubkey = "replacement-pubkey";
  resolveSave({ avatarUrl: INPUT.avatarUrl, pubkey: INPUT.expectedPubkey });
  await flushPromises();

  assert.deepEqual(harness.refreshed, []);
  assert.equal(harness.unsubscribeCount, 1);
});

test("retries a transient save and keeps the sync pending until success", async () => {
  const scheduled = [];
  let attempt = 0;
  const harness = createHarness({
    initialState: "ready",
    saveProfile: async (input) => {
      attempt += 1;
      if (attempt === 1) throw new Error("relay unreachable: network error");
      return { avatarUrl: input.avatarUrl, pubkey: input.expectedPubkey };
    },
    scheduleRetry: (callback, delayMs) => {
      const retry = { callback, delayMs, cancelled: false };
      scheduled.push(retry);
      return () => {
        retry.cancelled = true;
      };
    },
  });

  harness.sync.saveWhenReady(INPUT);
  await flushPromises();

  assert.equal(attempt, 1);
  assert.equal(harness.unsubscribeCount, 0);
  assert.equal(scheduled[0].delayMs, 5_000);

  harness.listener();
  await flushPromises();
  assert.equal(attempt, 1, "store updates must not bypass retry backoff");

  scheduled[0].callback();
  await flushPromises();

  assert.equal(attempt, 2);
  assert.equal(harness.refreshed.length, 1);
  assert.equal(harness.unsubscribeCount, 1);
});

test("reset cancels a scheduled transient retry", async () => {
  let scheduled;
  let attempts = 0;
  const harness = createHarness({
    initialState: "ready",
    saveProfile: async () => {
      attempts += 1;
      throw new Error("relay rate-limited: retry in 5s");
    },
    scheduleRetry: (callback, delayMs) => {
      scheduled = { callback, delayMs, cancelled: false };
      return () => {
        scheduled.cancelled = true;
      };
    },
  });

  harness.sync.saveWhenReady(INPUT);
  await flushPromises();
  harness.sync.reset();

  assert.equal(scheduled.delayMs, 5_000);
  assert.equal(scheduled.cancelled, true);
  scheduled.callback();
  await flushPromises();
  assert.equal(attempts, 1);
});

test("registration preserves readiness until the initial profile write completes", async () => {
  const harness = createHarness();
  const registration = harness.sync.registerWhenReady({
    avatarUrl: INPUT.avatarUrl,
    relayUrl: INPUT.relayUrl,
  });

  harness.setState("ready");
  harness.listener();
  harness.removePresentation();
  assert.deepEqual(harness.saves, []);

  registration.release({
    expectedPubkey: INPUT.expectedPubkey,
    expectedAvatarUrl: null,
  });
  await flushPromises();

  assert.deepEqual(harness.saves, [INPUT]);
  assert.equal(harness.refreshed.length, 1);
});

test("cancelled registration cannot save after the initial profile write fails", async () => {
  const harness = createHarness();
  const registration = harness.sync.registerWhenReady({
    avatarUrl: INPUT.avatarUrl,
    relayUrl: INPUT.relayUrl,
  });

  harness.setState("ready");
  harness.listener();
  registration.cancel();
  registration.release({
    expectedPubkey: INPUT.expectedPubkey,
    expectedAvatarUrl: null,
  });
  await flushPromises();

  assert.deepEqual(harness.saves, []);
  assert.equal(harness.unsubscribeCount, 1);
});

test("cache refresh waits for a provider and flushes exactly once", async () => {
  const refreshed = [];
  const queue = createProfileCacheRefreshQueue(
    async (client, profile, relayUrl) => {
      refreshed.push({ client, profile, relayUrl });
    },
  );
  const profile = { avatarUrl: INPUT.avatarUrl, pubkey: INPUT.expectedPubkey };
  const client = {};

  await queue.enqueue({ profile, input: INPUT });
  assert.deepEqual(refreshed, []);

  const detach = queue.setClient(client);
  await flushPromises();
  assert.deepEqual(refreshed, [{ client, profile, relayUrl: INPUT.relayUrl }]);

  detach();
  const detachAgain = queue.setClient(client);
  await flushPromises();
  assert.equal(refreshed.length, 1);
  detachAgain();
});

test("cache refresh reset discards work from the previous community", async () => {
  const refreshed = [];
  const queue = createProfileCacheRefreshQueue(
    async (client, profile, relayUrl) => {
      refreshed.push({ client, profile, relayUrl });
    },
  );

  await queue.enqueue({
    profile: { avatarUrl: INPUT.avatarUrl, pubkey: INPUT.expectedPubkey },
    input: INPUT,
  });
  queue.reset();
  queue.setClient({});
  await flushPromises();

  assert.deepEqual(refreshed, []);
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
