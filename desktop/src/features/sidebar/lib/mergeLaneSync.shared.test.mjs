// Shared parameterized test suite for MergeLaneSyncManager subclasses.
// Usage: import { runMergeLaneSyncSuite } from "./mergeLaneSync.shared.test.mjs";
//   runMergeLaneSyncSuite({ label: "stars", Manager: ..., ... });

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  installFakeWindow,
  installEchoTauri,
  installPresignTauriMock,
  makeFakeWindow,
  makeTimerBed,
} from "./sidebarSyncTestHelpers.mjs";
import { MergeLaneSyncManager } from "./mergeLaneSyncManager.ts";

function makeStore(channels = {}) {
  return { version: 1, channels };
}

export function runMergeLaneSyncSuite({
  label,
  Manager,
  readOutbox,
  watermarkKind,
  makeEntry,
  publish,
  getPending,
  fetchRemote,
}) {
  const RELAY = "wss://r.test";
  const RELAY_KEY = encodeURIComponent(RELAY);

  test(`${label}: observe: high-water is per-channel max of rev and updatedAt, monotonic`, () => {
    const fw = makeFakeWindow(),
      restore = installFakeWindow(fw);
    try {
      const m = new Manager("pk", RELAY);
      m.observe(
        makeStore({ a: makeEntry(true, 100, 3), b: makeEntry(false, 50, 1) }),
      );
      assert.equal(m.maxRevSeen("a"), 3);
      assert.equal(m.maxUpdatedAtSeen("a"), 100);
      m.observe(makeStore({ a: makeEntry(true, 90, 5) }));
      assert.equal(m.maxRevSeen("a"), 5, "rev raised");
      assert.equal(m.maxUpdatedAtSeen("a"), 100, "updatedAt not regressed");
      m.observe(makeStore({ a: makeEntry(true, 200, 2) }));
      assert.equal(m.maxUpdatedAtSeen("a"), 200, "updatedAt raised");
      assert.equal(m.maxRevSeen("a"), 5, "rev not regressed");
      assert.equal(m.maxRevSeen("never"), 0);
      assert.equal(m.maxUpdatedAtSeen("never"), 0);
    } finally {
      restore();
    }
  });

  test(`${label}: destroy: cancels pending publish; aborts in-flight publish after fetch resolves`, async () => {
    const publishCalls = [];
    mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
    mock.method(relayClient, "publishEvent", (...args) => {
      publishCalls.push(args);
      return Promise.resolve();
    });
    const fw = makeFakeWindow(),
      restore = installFakeWindow(fw);
    try {
      const manager = new Manager("pk-test", RELAY);
      publish(manager, makeStore({ ch1: makeEntry(true, 100, 1) }));
      manager.destroy();
      assert.equal(publishCalls.length, 0, "no publish after destroy");
      assert.equal(getPending(manager), null);
    } finally {
      restore();
      mock.reset();
    }
    let releaseFetch = null;
    const publishCalls2 = [];
    mock.method(
      relayClient,
      "fetchEvents",
      () =>
        new Promise((res) => {
          releaseFetch = () => res([]);
        }),
    );
    mock.method(relayClient, "publishEvent", (...args) => {
      publishCalls2.push(args);
      return Promise.resolve();
    });
    const fw2 = makeFakeWindow(),
      restore2 = installFakeWindow(fw2);
    try {
      const manager = new Manager("pk-race", RELAY);
      publish(manager, makeStore({ ch1: makeEntry(true, 100, 1) }));
      fw2._fireTimer();
      manager.destroy();
      releaseFetch();
      await new Promise((r) => setTimeout(r, 0));
      assert.equal(publishCalls2.length, 0);
    } finally {
      restore2();
      mock.reset();
    }
  });

  for (const { name, resolveFirst } of [
    {
      name: "A-succeeds",
      resolveFirst: (event, storedHead, res) => {
        storedHead.push(event);
        res();
      },
    },
    {
      name: "A-fails",
      resolveFirst: (_event, _storedHead, _res, rej) =>
        rej(new Error("socket error")),
    },
  ]) {
    test(`${label}: A-in-flight → B-click → ${name}: B stays pending and B publishes`, async () => {
      let releaseFirst = null,
        publishCount = 0;
      const storedHead = [];
      mock.method(relayClient, "fetchEvents", () =>
        Promise.resolve([...storedHead]),
      );
      mock.method(relayClient, "publishEvent", (event) => {
        publishCount++;
        if (publishCount === 1)
          return new Promise((res, rej) => {
            releaseFirst = () => resolveFirst(event, storedHead, res, rej);
          });
        storedHead.splice(0, storedHead.length, event);
        return Promise.resolve();
      });
      const t = makeTimerBed(),
        tauri = installEchoTauri(`pk-ab-${name}`);
      try {
        const manager = new Manager(`pk-ab-${name}`, RELAY);
        publish(manager, makeStore({ a: makeEntry(true, 100, 1) }));
        await t.fireDelay(2000);
        while (releaseFirst === null) await Promise.resolve();
        publish(manager, makeStore({ b: makeEntry(true, 101, 1) }));
        assert.deepEqual(
          Object.keys(getPending(manager).channels),
          ["b"],
          "B is now pending",
        );
        assert.ok(readOutbox(`pk-ab-${name}`, RELAY), "outbox holds B");
        releaseFirst();
        for (let i = 0; i < 50; i++) await Promise.resolve();
        assert.deepEqual(
          Object.keys(getPending(manager)?.channels ?? {}),
          ["b"],
          `${name}: B stays pending`,
        );
        assert.ok(
          readOutbox(`pk-ab-${name}`, RELAY),
          `${name}: B outbox intact`,
        );
        const capturedBefore = tauri.capturedPlaintext();
        await t.fireDelay(2000);
        for (let i = 0; i < 50; i++) await Promise.resolve();
        const captured = tauri.capturedPlaintext();
        assert.ok(
          captured && captured !== capturedBefore && captured.includes('"b"'),
          "B published",
        );
        assert.equal(
          getPending(manager),
          null,
          "B cleared after confirmed publish",
        );
        assert.equal(
          readOutbox(`pk-ab-${name}`, RELAY),
          null,
          "B outbox cleared",
        );
        manager.destroy();
      } finally {
        tauri.restore();
        t.restore();
        mock.reset();
      }
    });
  }

  test(`${label}: pre-sign guard: a newer edit during encrypt aborts the stale publish`, async () => {
    const publishCalls = [];
    mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
    mock.method(relayClient, "publishEvent", (...args) => {
      publishCalls.push(args);
      return Promise.resolve();
    });
    const presign = installPresignTauriMock("pk-presign"),
      fw = makeFakeWindow(),
      restore = installFakeWindow(fw);
    try {
      const manager = new Manager("pk-presign", RELAY);
      publish(manager, makeStore({ a: makeEntry(true, 100, 1) }));
      fw._fireTimer();
      while (presign.releaseEncrypt === null) await Promise.resolve();
      publish(manager, makeStore({ b: makeEntry(true, 101, 1) }));
      presign.releaseEncrypt();
      for (let i = 0; i < 50; i++) await Promise.resolve();
      assert.equal(
        publishCalls.length,
        0,
        "stale A must not publish after B queued",
      );
      assert.deepEqual(
        Object.keys(getPending(manager)?.channels ?? {}),
        ["b"],
        "B is still pending",
      );
      manager.destroy();
    } finally {
      presign.restore();
      restore();
      mock.reset();
    }
  });

  test(`${label}: failed publish schedules a bounded-backoff retry and keeps the pending edit`, async () => {
    let publishCount = 0,
      storedHead = [];
    mock.method(relayClient, "fetchEvents", () => Promise.resolve(storedHead));
    mock.method(relayClient, "publishEvent", (event) => {
      publishCount++;
      if (publishCount === 1) return Promise.reject(new Error("timeout"));
      storedHead = [event];
      return Promise.resolve();
    });
    const t = makeTimerBed(),
      tauri = installEchoTauri("pk-retry");
    try {
      const manager = new Manager("pk-retry", RELAY);
      publish(manager, makeStore({ a: makeEntry(true, 100, 1) }));
      await t.fireDelay(2000);
      assert.ok(
        getPending(manager) !== null,
        "pending edit retained after failure",
      );
      assert.ok(t.hasDelay(2000), "retry timer scheduled");
      await t.fireDelay(2000);
      for (let i = 0; i < 50; i++) await Promise.resolve();
      assert.equal(publishCount, 2, "retry re-published");
      assert.equal(
        getPending(manager),
        null,
        "pending cleared on retry success",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      t.restore();
      mock.reset();
    }
  });

  test(`${label}: publish OK but a peer blob is retained: loser keeps its outbox and retries`, async () => {
    let publishCount = 0,
      storedHead = [];
    mock.method(relayClient, "fetchEvents", () => Promise.resolve(storedHead));
    const tauri = installEchoTauri("pk-loser");
    mock.method(relayClient, "publishEvent", (event) => {
      publishCount++;
      if (publishCount === 1) {
        storedHead = [
          tauri.mintHead(makeStore({ z: makeEntry(true, 200, 5) }), 100),
        ];
        return Promise.resolve();
      }
      storedHead = [event];
      return Promise.resolve();
    });
    const t = makeTimerBed();
    try {
      const manager = new Manager("pk-loser", RELAY);
      publish(manager, makeStore({ a: makeEntry(true, 100, 1) }));
      await t.fireDelay(2000);
      for (let i = 0; i < 50; i++) await Promise.resolve();
      assert.ok(
        getPending(manager) !== null,
        "unconfirmed publish keeps pending edit",
      );
      assert.ok(readOutbox("pk-loser", RELAY), "loser keeps durable outbox");
      assert.ok(t.hasDelay(2000), "retry scheduled");
      await t.fireDelay(2000);
      for (let i = 0; i < 50; i++) await Promise.resolve();
      assert.equal(publishCount, 2, "loser retried");
      assert.equal(getPending(manager), null);
      manager.destroy();
    } finally {
      tauri.restore();
      t.restore();
      mock.reset();
    }
  });

  for (const {
    title,
    setupFetch,
    setupWatermark,
    relayOverride,
    pubkey,
    assertPending,
  } of [
    {
      title: "fetch error does not trigger seed-publish",
      setupFetch: () =>
        mock.method(relayClient, "fetchEvents", () =>
          Promise.reject(new Error("relay timeout")),
        ),
      assertPending: (m) => assert.equal(getPending(m), null),
    },
    {
      title: "absent fetch with prior watermark blocks seed-publish",
      setupFetch: () =>
        mock.method(relayClient, "fetchEvents", () => Promise.resolve([])),
      setupWatermark: (fw) =>
        fw.localStorage.setItem(
          `buzz-sync-watermark.v1:${watermarkKind}:pk-stale:${RELAY_KEY}`,
          "1700000000",
        ),
      pubkey: "pk-stale",
      assertPending: (m) => assert.equal(getPending(m), null),
    },
    {
      title: "absent fetch with zero watermark seeds (first-sync preserved)",
      setupFetch: () =>
        mock.method(relayClient, "fetchEvents", () => Promise.resolve([])),
      pubkey: "pk-fresh",
      assertPending: (m) => assert.ok(getPending(m) !== null),
    },
    {
      title: "relay-A watermark does not suppress first-sync seed on relay-B",
      setupFetch: () =>
        mock.method(relayClient, "fetchEvents", () => Promise.resolve([])),
      setupWatermark: (fw) =>
        fw.localStorage.setItem(
          `buzz-sync-watermark.v1:${watermarkKind}:pk-iso:${encodeURIComponent("wss://a.relay.test")}`,
          "1700000100",
        ),
      relayOverride: "wss://b.relay.test",
      pubkey: "pk-iso",
      assertPending: (m) =>
        assert.ok(
          getPending(m) !== null,
          "first-sync seed on relay B must not be blocked by relay A watermark",
        ),
    },
  ]) {
    test(`${label}: revert-fix: ${title}`, async () => {
      setupFetch();
      mock.method(relayClient, "publishEvent", () => Promise.resolve());
      const fw = makeFakeWindow();
      setupWatermark?.(fw);
      const restore = installFakeWindow(fw);
      try {
        const manager = new Manager(
          pubkey ?? "pk-fail",
          relayOverride ?? RELAY,
        );
        const result = await manager.bootstrap(
          makeStore({ ch1: makeEntry(true, 1, 0) }),
        );
        assert.equal(result.action, "hold");
        assertPending(manager);
      } finally {
        restore();
        mock.reset();
      }
    });
  }

  test(`${label}: timestamp clamp: published createdAt stays inside the relay future window`, async () => {
    const nowSecs = Math.floor(Date.now() / 1000);
    const fw = makeFakeWindow(),
      restore = installFakeWindow(fw),
      tauri = installEchoTauri("pk-clamp");
    const head = tauri.mintHead(makeStore({}), nowSecs + 3_600);
    mock.method(relayClient, "fetchEvents", () => Promise.resolve([head]));
    let signedCreatedAt = null;
    mock.method(relayClient, "publishEvent", (evt) => {
      signedCreatedAt = evt.created_at;
      return Promise.resolve();
    });
    try {
      const manager = new Manager("pk-clamp", RELAY);
      await fetchRemote(manager);
      publish(manager, makeStore({ ch1: makeEntry(true, 100, 1) }));
      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 20));
      assert.ok(signedCreatedAt !== null, "publish must have been attempted");
      assert.ok(
        signedCreatedAt <= Math.floor(Date.now() / 1000) + 840,
        `createdAt clamped — got ${signedCreatedAt}`,
      );
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}

const _synthOutbox = new Map();

function _parseStore(json) {
  if (!json || typeof json !== "object" || json.version !== 1) return null;
  if (typeof json.channels !== "object" || json.channels === null) return null;
  return json;
}

function _mergeStores(local, remote) {
  const merged = { version: 1, channels: { ...remote.channels } };
  for (const [id, le] of Object.entries(local.channels)) {
    const re = merged.channels[id];
    if (
      !re ||
      le.updatedAt > re.updatedAt ||
      (le.updatedAt === re.updatedAt && le.rev > re.rev)
    ) {
      merged.channels[id] = le;
    }
  }
  return merged;
}

function _isSubsumedBy(candidate, head) {
  return JSON.stringify(_mergeStores(head, candidate)) === JSON.stringify(head);
}

function _storesEqual(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

class SyntheticMergeLaneManager extends MergeLaneSyncManager {
  constructor(pubkey, relayUrl) {
    super(pubkey, relayUrl, {
      kind: 30078,
      dTag: "synth-merge-lane",
      logPrefix: "synth",
      publishTimeoutMsg: "synth timeout",
      publishErrorMsg: "synth error",
      parse: _parseStore,
      serializePayload: (s) => ({ version: 1, channels: s.channels }),
      mergeWithRemote: _mergeStores,
      isSubsumedBy: _isSubsumedBy,
      storesEqual: _storesEqual,
      observe: (hw, store) => {
        for (const [id, entry] of Object.entries(store.channels)) {
          const cur = hw.get(id) ?? { rev: 0, updatedAt: 0 };
          hw.set(id, {
            rev: Math.max(cur.rev, entry.rev ?? 0),
            updatedAt: Math.max(cur.updatedAt, entry.updatedAt ?? 0),
          });
        }
      },
      writeOutbox: (pubkey, store, relayUrl) => {
        _synthOutbox.set(`${pubkey}:${relayUrl}`, store);
      },
      clearOutbox: (pubkey, relayUrl) => {
        _synthOutbox.delete(`${pubkey}:${relayUrl}`);
      },
      isLocalNonEmpty: (s) => Object.keys(s.channels).length > 0,
    });
  }
}

runMergeLaneSyncSuite({
  label: "synth",
  Manager: SyntheticMergeLaneManager,
  readOutbox: (pubkey, relay) => _synthOutbox.get(`${pubkey}:${relay}`) ?? null,
  watermarkKind: "synth-merge-lane",
  makeEntry: (val, updatedAt, rev) => ({ val, updatedAt, rev }),
  publish: (m, s) => m.publish(s),
  getPending: (m) => m.getPendingStore(),
  fetchRemote: (m) => m.fetchRemoteBlob(),
});
