import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelSectionSyncManager } from "./channelSectionsSync.ts";

function makeStore(overrides = {}) {
  return {
    version: 1,
    sections: overrides.sections ?? [],
    assignments: overrides.assignments ?? {},
    ...overrides,
  };
}

// ─── Shared test helpers ───────────────────────────────────────────────────────

function makeFakeWindow() {
  const storage = new Map();
  const ls = {
    getItem: (k) => storage.get(k) ?? null,
    setItem: (k, v) => storage.set(k, v),
    removeItem: (k) => storage.delete(k),
    clear: () => storage.clear(),
  };
  let timerCallback = null;
  let nextTimerId = 100;
  return {
    localStorage: ls,
    setTimeout: (fn, _ms) => {
      timerCallback = fn;
      return nextTimerId++;
    },
    clearTimeout: (_id) => {
      timerCallback = null;
    },
    _fireTimer: () => {
      if (timerCallback) {
        const fn = timerCallback;
        timerCallback = null;
        fn();
      }
    },
    _hasTimer: () => timerCallback !== null,
  };
}

function installFakeWindow(fw) {
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  const origLs = globalThis.window.localStorage;
  const origSt = globalThis.window.setTimeout;
  const origCt = globalThis.window.clearTimeout;
  globalThis.window.localStorage = fw.localStorage;
  globalThis.window.setTimeout = fw.setTimeout;
  globalThis.window.clearTimeout = fw.clearTimeout;
  return () => {
    if (origLs !== undefined) globalThis.window.localStorage = origLs;
    if (origSt !== undefined) globalThis.window.setTimeout = origSt;
    if (origCt !== undefined) globalThis.window.clearTimeout = origCt;
  };
}

function makeSectionsStore(sections = []) {
  return { version: 1, sections, assignments: {} };
}

const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);

// ─── destroy() must cancel pending publish, not flush ─────────────────────────

// Regression guard for the community-switch cross-relay publish vector:
// edit sections in relay A → destroy() is called (relayUrl dep change) →
// no publish should fire.
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
    const manager = new ChannelSectionSyncManager("pk-test", RELAY);
    manager.publishSections(makeStore({ sections: [{ id: "s1", name: "Work", order: 0 }] }));
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
  mock.method(relayClient, "fetchEvents", () => new Promise((res) => { releaseFetch = () => res([]); }));
  mock.method(relayClient, "publishEvent", (...args) => { publishCalls.push(args); return Promise.resolve(); });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-race", RELAY);
    manager.publishSections(makeStore({ sections: [{ id: "s1", name: "Work", order: 0 }] }));
    fw._fireTimer(); // starts doPublish, which is now awaiting fetchOwnBlobBeforePublish
    manager.destroy();
    releaseFetch();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(publishCalls.length, 0, "publishEvent must not fire after destroy");
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-no-pending", RELAY);
    assert.doesNotThrow(() => manager.destroy());
  } finally {
    restore();
  }
});

// ─── Boot seed-publish guard (the revert-fix regression suite) ────────────────
// Wiring tests 1-3 drive the production bootstrap() path; policy tested once
// in sidebarSyncWatermark.test.mjs.

// 1. fetch failed → hold, pendingStore null (mutation: remove failed guard → seed queued)
test("revert-fix: fetch failed (error) does not trigger seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.reject(new Error("relay timeout")));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-fail", RELAY);
    const result = await manager.bootstrap(makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]));
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. absent + prior watermark → hold, pendingStore null (mutation: clear watermark → seed queued)
test("revert-fix: absent fetch with prior watermark blocks seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  fw.localStorage.setItem(`buzz-sync-watermark.v1:channel-sections:pk-stale:${RELAY_KEY}`, "1700000000");
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-stale", RELAY);
    assert.ok(manager.getPersistedWatermark() > 0);
    const result = await manager.bootstrap(makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]));
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. absent + zero watermark + non-empty → seed queued (mutation: remove seed call → pendingStore null)
test("revert-fix: absent fetch with zero watermark seeds via bootstrap (first-sync preserved)", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-fresh", RELAY);
    const result = await manager.bootstrap(makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]));
    assert.equal(result.action, "hold");
    assert.ok(manager.getPendingStore() !== null);
  } finally {
    restore();
    mock.reset();
  }
});

// 4. LWW baseline: newer decryptable pre-publish event still wins after an
//    undecryptable head was recorded.
// Mutation test: removing headBeforeFetch snapshot causes remote to never win.
test("revert-fix: sections LWW — newer decryptable pre-publish event selected after undecryptable head recorded", async () => {
  // Boot fetch: undecryptable event, created_at=100 → head recorded to 100.
  // Pre-publish fetch: decryptable event, created_at=200 → should win (200 > 100).
  // Mutation: if headBeforeFetch is dropped and this.lastRemoteCreatedAt used instead,
  // the comparison becomes 200 > 200 = false → local wins instead of remote → wrong content encrypted.
  const REMOTE_SECTION_ID = "remote-section-from-relay";
  const LOCAL_SECTION_ID = "local-section-from-app";
  let capturedEncryptPlaintext = null;
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

  // Intercept Tauri invokes so decryptAndParse, nip44EncryptToSelf, and signRelayEvent work in Node.
  const origTauri = globalThis.window?.__TAURI_INTERNALS__;
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      if (cmd === "nip44_decrypt_from_self") {
        // "bad-cipher" fails; "good-cipher" returns a valid sections payload.
        if (args?.ciphertext === "bad-cipher") return Promise.reject(new Error("decrypt failed"));
        const remotePayload = JSON.stringify({
          version: 1,
          sections: [{ id: REMOTE_SECTION_ID, name: "Remote", order: 0 }],
          assignments: {},
        });
        return Promise.resolve(remotePayload);
      }
      if (cmd === "nip44_encrypt_to_self") {
        capturedEncryptPlaintext = args?.plaintext ?? null;
        return Promise.resolve("encrypted-ciphertext");
      }
      if (cmd === "sign_event") {
        return Promise.resolve(JSON.stringify({
          id: "signed-event-id",
          pubkey: "pk-lww",
          content: "encrypted-ciphertext",
          created_at: args?.createdAt ?? 999,
          kind: args?.kind ?? 0,
          tags: args?.tags ?? [],
          sig: "fake-sig",
        }));
      }
      return Promise.reject(new Error(`unmocked tauri: ${cmd}`));
    },
  };

  try {
    const manager = new ChannelSectionSyncManager("pk-lww", RELAY);
    // Boot fetch: sees event@100 (bad-cipher), records head to 100. Remote = null.
    await manager.fetchRemoteSections();
    assert.ok(manager.getPersistedWatermark() >= 100, "head must be recorded from boot event");

    // Queue a publish with local sections — triggers doPublish → fetchOwnBlobBeforePublish (callCount=2).
    const localStore = makeSectionsStore([{ id: LOCAL_SECTION_ID, name: "Local", order: 0 }]);
    manager.publishSections(localStore);
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));

    // If headBeforeFetch is correctly snapshotted: remote.createdAt(200) > headBeforeFetch(100) → true
    //   → fetchOwnBlobBeforePublish returns remote.store → nip44_encrypt_to_self gets remote sections.
    // If NOT snapshotted (mutation): 200 > this.lastRemoteCreatedAt(200) → false
    //   → returns local store → nip44_encrypt_to_self gets local sections.
    assert.ok(capturedEncryptPlaintext !== null, "nip44EncryptToSelf must have been called");
    const encrypted = JSON.parse(capturedEncryptPlaintext);
    assert.ok(
      Array.isArray(encrypted.sections) && encrypted.sections.some((s) => s.id === REMOTE_SECTION_ID),
      `remote sections must win LWW merge — got: ${capturedEncryptPlaintext}`,
    );
  } finally {
    if (origTauri !== undefined) {
      globalThis.window.__TAURI_INTERNALS__ = origTauri;
    } else {
      delete globalThis.window.__TAURI_INTERNALS__;
    }
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
    const manager = new ChannelSectionSyncManager("pk-live", RELAY);
    assert.equal(manager.getPersistedWatermark(), 0, "watermark starts at 0");
    await manager.subscribeToSections(() => {});
    assert.ok(liveCallback !== null, "subscribeLive must have captured the callback");
    liveCallback({ pubkey: "pk-live", content: "!bad-cipher!", created_at: 1700005555, id: "live-evt-1" });
    await new Promise((r) => setTimeout(r, 0));
    assert.ok(
      manager.getPersistedWatermark() >= 1700005555,
      "live undecryptable event must advance the watermark before decrypt is attempted",
    );
  } finally {
    restore();
    mock.reset();
  }
});
