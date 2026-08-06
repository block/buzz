import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelSortSyncManager } from "./channelSortSync.ts";
import {
  makeFakeWindow,
  installFakeWindow,
  installTauriMock,
} from "./sidebarSyncTestHelpers.mjs";

function makeStore(groups = {}) {
  return { version: 1, groups };
}

const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);

// ─── destroy() must cancel pending publish, not flush ─────────────────────────
test("destroy: cancels pending publish without flushing to the relay", () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  const publishCalls = [];
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-test", RELAY);
    manager.publishSortPrefs(makeStore({ channels: "recent" }));
    assert.ok(fw._hasTimer(), "debounce timer should be set");
    manager.destroy();
    assert.ok(!fw._hasTimer(), "debounce timer should be cleared on destroy");
    assert.equal(publishCalls.length, 0);
    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// Regression guard for the timer-fired race: debounce fires → doPublish awaits
// fetchOwnBlobBeforePublish → destroy() called → publishEvent must not fire.
test("destroy: aborts in-flight doPublish after fetchOwnBlobBeforePublish resolves", async () => {
  let releaseFetch = null;
  const publishCalls = [];
  mock.method(
    relayClient,
    "fetchEvents",
    () =>
      new Promise((res) => {
        releaseFetch = () => res([]);
      }),
  );
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-race", RELAY);
    manager.publishSortPrefs(makeStore({ dms: "recent" }));
    fw._fireTimer(); // starts doPublish, which is now awaiting fetchOwnBlobBeforePublish
    manager.destroy();
    releaseFetch();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(
      publishCalls.length,
      0,
      "publishEvent must not fire after destroy",
    );
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-no-pending", RELAY);
    assert.doesNotThrow(() => manager.destroy());
  } finally {
    restore();
  }
});

// ─── Boot seed-publish guard (the revert-fix regression suite) ────────────────
// Wiring tests 1-3 drive the production bootstrap() path; policy tested once
// in sidebarSyncWatermark.test.mjs.

// 1. fetch failed (error/timeout) + local non-empty → hold, zero publish calls
// Mutation: removing the failed guard causes bootstrap to call publishSortPrefs → pendingStore set.
test("revert-fix: fetch failed (error) does not trigger seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("relay timeout")),
  );
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-fail", RELAY);
    const result = await manager.bootstrap(makeStore({ channels: "recent" }));
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. absent + persisted head > 0 → hold, zero publish calls (the dev-build stale-copy case)
// Mutation: setting watermark to 0 in localStorage causes bootstrap to seed.
test("revert-fix: absent fetch with prior watermark blocks seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-sort:pk-stale:${RELAY_KEY}`,
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-stale", RELAY);
    assert.ok(
      Number(
        fw.localStorage.getItem(
          `buzz-sync-watermark.v1:channel-sort:pk-stale:${RELAY_KEY}`,
        ) ?? "0",
      ) > 0,
    );
    const result = await manager.bootstrap(makeStore({ channels: "recent" }));
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. absent + head 0 + local non-empty → seed-publish queued (first-sync preserved)
// Mutation: removing the absent+head-0 seed call leaves pendingStore null.
test("revert-fix: absent fetch with zero watermark seeds via bootstrap (first-sync preserved)", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-fresh", RELAY);
    assert.equal(
      fw.localStorage.getItem(
        `buzz-sync-watermark.v1:channel-sort:pk-fresh:${RELAY_KEY}`,
      ),
      null,
    );
    const result = await manager.bootstrap(makeStore({ channels: "recent" }));
    assert.equal(result.action, "hold");
    assert.ok(manager.getPendingStore() !== null);
  } finally {
    restore();
    mock.reset();
  }
});

// 4. LWW baseline: newer decryptable pre-publish event still wins after an
//    undecryptable head was recorded.
// Mutation test: headBeforeFetch → this.lastRemoteCreatedAt makes comparison
// 200>200=false → local wins instead of remote → wrong content encrypted.
test("revert-fix: sort LWW — newer decryptable pre-publish event selected after undecryptable head recorded", async () => {
  const REMOTE_KEY = "remote-group-from-relay";
  let callCount = 0;
  mock.method(relayClient, "fetchEvents", () => {
    callCount++;
    return Promise.resolve([
      {
        pubkey: "pk-lww",
        content: callCount === 1 ? "bad-cipher" : "good-cipher",
        created_at: callCount === 1 ? 100 : 200,
        id: `evt-${callCount}`,
      },
    ]);
  });
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installTauriMock(
    JSON.stringify({ version: 1, groups: { [REMOTE_KEY]: "recent" } }),
  );
  try {
    const manager = new ChannelSortSyncManager("pk-lww", RELAY);
    await manager.fetchRemoteSortPrefs();
    assert.ok(
      Number(
        fw.localStorage.getItem(
          `buzz-sync-watermark.v1:channel-sort:pk-lww:${RELAY_KEY}`,
        ) ?? "0",
      ) >= 100,
    );
    manager.publishSortPrefs(makeStore({ "local-group": "recent" }));
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    const pt = tauri.capturedPlaintext();
    assert.ok(pt !== null, "nip44EncryptToSelf must have been called");
    assert.ok(
      JSON.parse(pt).groups && REMOTE_KEY in JSON.parse(pt).groups,
      `remote groups must win LWW merge — got: ${pt}`,
    );
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

// 5. live-sub: undecryptable event on live path records head before decrypt
// Mutation test: removing recordRemoteHead before decrypt in the live callback
// leaves watermark at 0 after a live event.
test("revert-fix: undecryptable live event advances watermark before decrypt attempt", async () => {
  let liveCallback = null;
  mock.method(relayClient, "subscribeLive", (_filter, onEvent) => {
    liveCallback = onEvent;
    return Promise.resolve(async () => {});
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-live", RELAY);
    assert.equal(
      fw.localStorage.getItem(
        `buzz-sync-watermark.v1:channel-sort:pk-live:${RELAY_KEY}`,
      ),
      null,
      "watermark starts absent",
    );
    await manager.subscribeToSortPrefs(() => {});
    assert.ok(
      liveCallback !== null,
      "subscribeLive must have captured the callback",
    );
    liveCallback({
      pubkey: "pk-live",
      content: "!bad-cipher!",
      created_at: 1700005555,
      id: "live-evt-1",
    });
    await new Promise((r) => setTimeout(r, 0));
    assert.ok(
      Number(
        fw.localStorage.getItem(
          `buzz-sync-watermark.v1:channel-sort:pk-live:${RELAY_KEY}`,
        ) ?? "0",
      ) >= 1700005555,
      "live undecryptable event must advance the watermark before decrypt is attempted",
    );
  } finally {
    restore();
    mock.reset();
  }
});
