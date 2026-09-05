// Concrete adapter-contract test helpers for the two sidebar sync lanes.
//
// Exports:
//   runSectionsAdapterContract() — sections-lane concrete adapter contract
//   runSortAdapterContract()     — sort-lane concrete adapter contract
//
// Re-exported by sidebarSyncTestHelpers.mjs so callers importing from there
// are unaffected by the split.

import { ChannelSectionSyncManager } from "./channelSectionsSync.ts";
import {
  readChannelSectionsOutbox,
  writeChannelSectionsOutbox,
} from "./channelSectionsStorage.ts";
import { ChannelSortSyncManager } from "./channelSortSync.ts";
import {
  readChannelSortOutbox,
  writeChannelSortOutbox,
} from "./channelSortPreference.ts";
import {
  makeFakeWindow,
  installFakeWindow,
  installEchoTauri,
  installTauriMock,
} from "./sidebarSyncTestHelpers.mjs";

// ---------------------------------------------------------------------------
// Concrete exported lane adapter contracts — sections and sort.
//
// Each function registers three node:test tests for its lane. No lane
// descriptor object; lane-specific values are inlined directly.
//
// Shared adapter infrastructure (not lane-specific):
//   ADAPTER_RELAY           — relay URL constant used by all adapter tests
//   _adapterCtx()           — lazily imports and caches { assert, test, mock,
//                             relayClient }; called once per lane runner and
//                             once per phase-helper invocation
//   assertExactFilter(...)  — 5-scalar relay filter deep-equal
//   _runDurableResumeTest(...)     — lane-typed durable-resume phase
//   _runUnsupportedVersionTest(...)— lane-typed unsupported-version phase
// ---------------------------------------------------------------------------

const ADAPTER_RELAY = "wss://r.test";

let _adapterCtxCache = null;
async function _adapterCtx() {
  if (_adapterCtxCache) return _adapterCtxCache;
  const { default: assert } = await import("node:assert/strict");
  const { default: test, mock } = await import("node:test");
  const { relayClient } = await import("@/shared/api/relayClient");
  _adapterCtxCache = { assert, test, mock, relayClient };
  return _adapterCtxCache;
}

// assertExactFilter — deep-equals a captured relay filter against the five
// required scalar fields (kinds, authors, #d, limit). Used by both lanes.
async function assertExactFilter(
  assert,
  captured,
  kind,
  dTag,
  pubkey,
  limit,
  msg,
) {
  assert.deepEqual(
    captured,
    { kinds: [kind], authors: [pubkey], "#d": [dTag], limit },
    msg,
  );
}

// _runDurableResumeTest — verifies the outbox write survives a manager
// remount and the resumed edit publishes exactly once.
// Lane-specific params: laneLabel, Manager class, publishTo closure,
// readOutbox fn, makeStore fn. Infrastructure via _adapterCtx().
async function _runDurableResumeTest(
  laneLabel,
  Manager,
  publishTo,
  readOutbox,
  makeStore,
) {
  const { assert, test, mock, relayClient } = await _adapterCtx();
  test(`${laneLabel}: durable outbox — edit persisted and resumed across remount`, async () => {
    let storedHead = [];
    mock.method(relayClient, "fetchEvents", () => Promise.resolve(storedHead));
    const publishCalls = [];
    mock.method(relayClient, "publishEvent", (event) => {
      publishCalls.push(event);
      storedHead = [event];
      return Promise.resolve();
    });
    const fw = makeFakeWindow();
    const restore = installFakeWindow(fw);
    const tauri = installEchoTauri(`pk-resume-${laneLabel}`);
    const pubkey = `pk-resume-${laneLabel}`;
    try {
      const m1 = new Manager(pubkey, ADAPTER_RELAY);
      publishTo(m1, makeStore());
      assert.ok(
        readOutbox(pubkey, ADAPTER_RELAY) !== null,
        "edit persisted before teardown",
      );
      m1.destroy();
      assert.equal(publishCalls.length, 0, "destroy must not flush");
      const m2 = new Manager(pubkey, ADAPTER_RELAY);
      const persisted = readOutbox(pubkey, ADAPTER_RELAY);
      publishTo(m2, persisted.store);
      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 20));
      assert.equal(publishCalls.length, 1, "resumed edit must publish");
      assert.equal(
        readOutbox(pubkey, ADAPTER_RELAY),
        null,
        "outbox cleared after confirm",
      );
      m2.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}

// _runUnsupportedVersionTest — verifies an unsupported relay head blocks
// publish and retains the pending edit.
// Lane-specific params: laneLabel, Manager class, publishTo closure,
// makeStore fn, badVersionPayload string. Infrastructure via _adapterCtx().
async function _runUnsupportedVersionTest(
  laneLabel,
  Manager,
  publishTo,
  makeStore,
  badVersionPayload,
) {
  const { assert, test, mock, relayClient } = await _adapterCtx();
  test(`${laneLabel}: unsupported head version retains pending edit, never publishing`, async () => {
    mock.method(relayClient, "fetchEvents", () =>
      Promise.resolve([
        {
          pubkey: `pk-bv-${laneLabel}`,
          content: "good-cipher",
          created_at: 500,
          id: "evt",
        },
      ]),
    );
    const publishCalls = [];
    mock.method(relayClient, "publishEvent", (...args) => {
      publishCalls.push(args);
      return Promise.resolve();
    });
    const fw = makeFakeWindow();
    const restore = installFakeWindow(fw);
    const tauri = installTauriMock(badVersionPayload);
    let manager = null;
    try {
      manager = new Manager(`pk-bv-${laneLabel}`, ADAPTER_RELAY);
      publishTo(manager, makeStore());
      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 20));
      assert.equal(
        publishCalls.length,
        0,
        "must not publish over unsupported head",
      );
      assert.ok(manager.getPendingStore() !== null, "pending edit retained");
      assert.ok(fw._hasTimer(), "retry scheduled");
    } finally {
      manager?.destroy();
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}

export async function runSectionsAdapterContract() {
  const { assert, test, mock, relayClient } = await _adapterCtx();
  const KIND = 30078;
  const DTAG = "channel-sections";

  function makeStore(sections = []) {
    return { version: 1, sections, assignments: {} };
  }

  // ── Wiring + outbox write/clear + opposite-lane isolation ──────────────
  test(`sections: kind=${KIND}, d-tag='${DTAG}', payload key 'sections'; write-lane and clear-lane isolation`, async () => {
    let storedHead = [];
    let liveCb = null;
    let capturedFetchFilter = null;
    let capturedLiveFilter = null;
    mock.method(relayClient, "fetchEvents", (f) => {
      capturedFetchFilter = f;
      return Promise.resolve(storedHead);
    });
    mock.method(relayClient, "subscribeLive", (f, cb) => {
      capturedLiveFilter = f;
      liveCb = cb;
      return Promise.resolve(async () => {});
    });
    let publishedEvent = null;
    mock.method(relayClient, "publishEvent", (evt) => {
      publishedEvent = evt;
      storedHead = [evt];
      return Promise.resolve();
    });
    const fw = makeFakeWindow();
    const restore = installFakeWindow(fw);
    const tauri = installEchoTauri("pk-wire-sections");
    const pubkey = "pk-wire-sections";
    try {
      // Seed sort-lane sentinel; sections clear must not touch it.
      writeChannelSortOutbox(
        pubkey,
        { version: 1, groups: { channels: "recent" } },
        ADAPTER_RELAY,
      );
      const otherBefore = readChannelSortOutbox(pubkey, ADAPTER_RELAY);
      assert.ok(otherBefore !== null, "opposite-lane (sort) sentinel seeded");
      const sentinelStore = otherBefore.store;

      const m = new ChannelSectionSyncManager(pubkey, ADAPTER_RELAY);
      m.publishSections(makeStore([{ id: "s1", name: "S", order: 0 }]));

      // Write-lane isolation.
      assert.ok(
        readChannelSectionsOutbox(pubkey, ADAPTER_RELAY) !== null,
        "sections outbox written — cross-lane write wires wrong outbox",
      );
      {
        const otherAfterWrite = readChannelSortOutbox(pubkey, ADAPTER_RELAY);
        assert.ok(
          otherAfterWrite !== null,
          "opposite-lane sentinel present after own write — cross-lane write breaks this",
        );
        assert.deepEqual(
          otherAfterWrite.store,
          sentinelStore,
          "opposite-lane sentinel store unchanged after own write",
        );
      }

      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 20));

      assert.ok(publishedEvent !== null, "publish fired");
      assert.equal(publishedEvent.kind, KIND, `kind=${KIND}`);
      const dTag = publishedEvent.tags.find((t) => t[0] === "d")?.[1];
      assert.equal(dTag, DTAG, `d-tag='${DTAG}'`);
      const parsed = JSON.parse(tauri.capturedPlaintext());
      assert.ok("sections" in parsed, "payload has 'sections'");
      assert.ok(!("groups" in parsed), "payload has no 'groups'");
      assert.ok(!("channels" in parsed), "payload has no 'channels'");

      // Clear-lane isolation: check opposite sentinel BEFORE own-outbox-cleared.
      {
        const otherAfterClear = readChannelSortOutbox(pubkey, ADAPTER_RELAY);
        assert.ok(
          otherAfterClear !== null,
          "opposite-lane sentinel survives own-lane clear — cross-lane clear breaks this",
        );
        assert.deepEqual(
          otherAfterClear.store,
          sentinelStore,
          "opposite-lane sentinel store unchanged after own-lane clear — cross-lane clear drops this",
        );
      }
      assert.equal(
        readChannelSectionsOutbox(pubkey, ADAPTER_RELAY),
        null,
        "own outbox cleared after confirm",
      );

      // Typed fetch: exact filter + full store.
      const fetchResult = await m.fetchRemoteSections();
      assert.equal(
        fetchResult.status,
        "found",
        "fetchRemoteSections returns found",
      );
      await assertExactFilter(
        assert,
        capturedFetchFilter,
        KIND,
        DTAG,
        pubkey,
        1,
        "fetch filter exact object: kinds/authors/#d/limit",
      );
      assert.deepEqual(
        fetchResult.data.store,
        makeStore([{ id: "s1", name: "S", order: 0 }]),
        "fetchRemoteSections returns exact published store",
      );

      // Typed subscribe: exact live filter + disposer identity + callback payload.
      const rcvd = [];
      const disposerSentinel = async () => {
        disposerSentinel._called = true;
      };
      const origSub = relayClient.subscribeLive;
      relayClient.subscribeLive = (f, cb) => {
        capturedLiveFilter = f;
        liveCb = cb;
        return Promise.resolve(disposerSentinel);
      };
      const returnedDisposer = await m.subscribeToSections((r) => rcvd.push(r));
      relayClient.subscribeLive = origSub;

      assert.strictEqual(
        returnedDisposer,
        disposerSentinel,
        "subscribeToSections returns the disposer from subscribeLive",
      );
      await assertExactFilter(
        assert,
        capturedLiveFilter,
        KIND,
        DTAG,
        pubkey,
        0,
        "live filter exact object: kinds/authors/#d/limit",
      );

      const liveHead = tauri.mintHead(
        makeStore([{ id: "p", name: "S", order: 0 }]),
        (storedHead[0]?.created_at ?? 0) + 1,
        "live",
      );
      liveCb(liveHead);
      await new Promise((r) => setTimeout(r, 20));
      assert.equal(rcvd.length, 1, "subscribeToSections delivers live update");
      assert.deepEqual(
        rcvd[0].store,
        makeStore([{ id: "p", name: "S", order: 0 }]),
        "callback delivers exact live sections store (full, including assignments)",
      );

      await returnedDisposer();
      assert.equal(
        returnedDisposer._called,
        true,
        "invoking returned disposer calls through",
      );

      m.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // ── Durable outbox — resume across remount ──────────────────────────────
  await _runDurableResumeTest(
    "sections",
    ChannelSectionSyncManager,
    (m, s) => m.publishSections(s),
    readChannelSectionsOutbox,
    () => makeStore([{ id: "s1", name: "S", order: 0 }]),
  );

  // ── Unsupported-version head retains pending edit ────────────────────────
  await _runUnsupportedVersionTest(
    "sections",
    ChannelSectionSyncManager,
    (m, s) => m.publishSections(s),
    () => makeStore([{ id: "s1", name: "S", order: 0 }]),
    JSON.stringify({ version: 2, sections: [], assignments: {} }),
  );
}

export async function runSortAdapterContract() {
  const { assert, test, mock, relayClient } = await _adapterCtx();
  const KIND = 30078;
  const DTAG = "channel-sort";

  function makeStore(groups = []) {
    const obj = {};
    for (const g of groups) obj[g.id] = "recent";
    return { version: 1, groups: obj };
  }

  // ── Wiring + outbox write/clear + opposite-lane isolation ──────────────
  test(`sort: kind=${KIND}, d-tag='${DTAG}', payload key 'groups'; write-lane and clear-lane isolation`, async () => {
    let storedHead = [];
    let liveCb = null;
    let capturedFetchFilter = null;
    let capturedLiveFilter = null;
    mock.method(relayClient, "fetchEvents", (f) => {
      capturedFetchFilter = f;
      return Promise.resolve(storedHead);
    });
    mock.method(relayClient, "subscribeLive", (f, cb) => {
      capturedLiveFilter = f;
      liveCb = cb;
      return Promise.resolve(async () => {});
    });
    let publishedEvent = null;
    mock.method(relayClient, "publishEvent", (evt) => {
      publishedEvent = evt;
      storedHead = [evt];
      return Promise.resolve();
    });
    const fw = makeFakeWindow();
    const restore = installFakeWindow(fw);
    const tauri = installEchoTauri("pk-wire-sort");
    const pubkey = "pk-wire-sort";
    try {
      // Seed sections-lane sentinel; sort clear must not touch it.
      writeChannelSectionsOutbox(
        pubkey,
        {
          version: 1,
          sections: [{ id: "sentinel", name: "S", order: 0 }],
          assignments: {},
        },
        ADAPTER_RELAY,
      );
      const otherBefore = readChannelSectionsOutbox(pubkey, ADAPTER_RELAY);
      assert.ok(
        otherBefore !== null,
        "opposite-lane (sections) sentinel seeded",
      );
      const sentinelStore = otherBefore.store;

      const m = new ChannelSortSyncManager(pubkey, ADAPTER_RELAY);
      m.publishSortPrefs(makeStore([{ id: "p" }]));

      // Write-lane isolation.
      assert.ok(
        readChannelSortOutbox(pubkey, ADAPTER_RELAY) !== null,
        "sort outbox written — cross-lane write wires wrong outbox",
      );
      {
        const otherAfterWrite = readChannelSectionsOutbox(
          pubkey,
          ADAPTER_RELAY,
        );
        assert.ok(
          otherAfterWrite !== null,
          "opposite-lane sentinel present after own write — cross-lane write breaks this",
        );
        assert.deepEqual(
          otherAfterWrite.store,
          sentinelStore,
          "opposite-lane sentinel store unchanged after own write",
        );
      }

      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 20));

      assert.ok(publishedEvent !== null, "publish fired");
      assert.equal(publishedEvent.kind, KIND, `kind=${KIND}`);
      const dTag = publishedEvent.tags.find((t) => t[0] === "d")?.[1];
      assert.equal(dTag, DTAG, `d-tag='${DTAG}'`);
      const parsed = JSON.parse(tauri.capturedPlaintext());
      assert.ok("groups" in parsed, "payload has 'groups'");
      assert.ok(!("sections" in parsed), "payload has no 'sections'");
      assert.ok(!("channels" in parsed), "payload has no 'channels'");

      // Clear-lane isolation: check opposite sentinel BEFORE own-outbox-cleared.
      {
        const otherAfterClear = readChannelSectionsOutbox(
          pubkey,
          ADAPTER_RELAY,
        );
        assert.ok(
          otherAfterClear !== null,
          "opposite-lane sentinel survives own-lane clear — cross-lane clear breaks this",
        );
        assert.deepEqual(
          otherAfterClear.store,
          sentinelStore,
          "opposite-lane sentinel store unchanged after own-lane clear — cross-lane clear drops this",
        );
      }
      assert.equal(
        readChannelSortOutbox(pubkey, ADAPTER_RELAY),
        null,
        "own outbox cleared after confirm",
      );

      // Typed fetch: exact filter + full store.
      const fetchResult = await m.fetchRemoteSortPrefs();
      assert.equal(
        fetchResult.status,
        "found",
        "fetchRemoteSortPrefs returns found",
      );
      await assertExactFilter(
        assert,
        capturedFetchFilter,
        KIND,
        DTAG,
        pubkey,
        1,
        "fetch filter exact object: kinds/authors/#d/limit",
      );
      assert.deepEqual(
        fetchResult.data.store,
        makeStore([{ id: "p" }]),
        "fetchRemoteSortPrefs returns exact published store (groups.p === 'recent')",
      );

      // Typed subscribe: exact live filter + disposer identity + callback payload.
      const rcvd = [];
      const disposerSentinel = async () => {
        disposerSentinel._called = true;
      };
      const origSub = relayClient.subscribeLive;
      relayClient.subscribeLive = (f, cb) => {
        capturedLiveFilter = f;
        liveCb = cb;
        return Promise.resolve(disposerSentinel);
      };
      const returnedDisposer = await m.subscribeToSortPrefs((r) =>
        rcvd.push(r),
      );
      relayClient.subscribeLive = origSub;

      assert.strictEqual(
        returnedDisposer,
        disposerSentinel,
        "subscribeToSortPrefs returns the disposer from subscribeLive",
      );
      await assertExactFilter(
        assert,
        capturedLiveFilter,
        KIND,
        DTAG,
        pubkey,
        0,
        "live filter exact object: kinds/authors/#d/limit",
      );

      const liveHead = tauri.mintHead(
        makeStore([{ id: "p" }]),
        (storedHead[0]?.created_at ?? 0) + 1,
        "live",
      );
      liveCb(liveHead);
      await new Promise((r) => setTimeout(r, 20));
      assert.equal(rcvd.length, 1, "subscribeToSortPrefs delivers live update");
      assert.deepEqual(
        rcvd[0].store,
        makeStore([{ id: "p" }]),
        "callback delivers exact live sort store (groups.p === 'recent')",
      );

      await returnedDisposer();
      assert.equal(
        returnedDisposer._called,
        true,
        "invoking returned disposer calls through",
      );

      m.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // ── Durable outbox — resume across remount ──────────────────────────────
  await _runDurableResumeTest(
    "sort",
    ChannelSortSyncManager,
    (m, s) => m.publishSortPrefs(s),
    readChannelSortOutbox,
    () => makeStore([{ id: "s1" }]),
  );

  // ── Unsupported-version head retains pending edit ────────────────────────
  await _runUnsupportedVersionTest(
    "sort",
    ChannelSortSyncManager,
    (m, s) => m.publishSortPrefs(s),
    () => makeStore([{ id: "s1" }]),
    JSON.stringify({ version: 2, groups: {} }),
  );
}
