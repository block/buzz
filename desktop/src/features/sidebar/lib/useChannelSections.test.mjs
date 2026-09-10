import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { makeHookStubs } from "./sidebarSyncTestHelpers.mjs";
import { runWholeBlobHookSuite } from "./wholeBlobHook.shared.test.mjs";
import { runWholeBlobP2aSuite } from "./wholeBlobSyncP2a.shared.test.mjs";
import {
  runWholeBlobCarlSuite,
  runWholeBlobC2Suite,
  runWholeBlobC3Suite,
  runWholeBlobP2a1Suite,
  runWholeBlobP2a1HookSuite,
  runWholeBlobP2bSuite,
} from "./wholeBlobSyncCarl.shared.test.mjs";
import { runWholeBlobCarlP2SecondClickSuite } from "./wholeBlobSyncCarlP2SecondClick.shared.test.mjs";

const { act, cleanup, renderHook } = await import("@testing-library/react");
const { relayClient } = await import("@/shared/api/relayClient");
const {
  MAX_CHANNEL_SECTION_ASSIGNMENTS,
  storageKey,
  readChannelSectionsOutbox,
} = await import("./channelSectionsStorage.ts");
const { useChannelSections } = await import("./useChannelSections.ts");
const { ChannelSectionSyncManager } = await import("./channelSectionsSync.ts");
const { normalizeRelayUrl } = await import("@/shared/lib/normalizeRelayUrl");

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});
after(() => dom.window.close());

const { stubRelay, stubTauri } = makeHookStubs();

runWholeBlobHookSuite({
  label: "sections",
  dTag: "channel-sections",
  useHook: useChannelSections,
  storageKey,
  readOutbox: readChannelSectionsOutbox,
  legacyOutboxKey: (pubkey, relayUrl) => {
    const encoded = encodeURIComponent(
      relayUrl.trim().replace(/\/$/, "").toLowerCase(),
    );
    return `buzz-channel-sections-outbox.v1:${pubkey}:${encoded}`;
  },
  makeEdit: (r) => r.createSection("A1-Section"),
  makeB1Store: () =>
    JSON.stringify({
      version: 1,
      sections: [{ id: "b1-section", name: "B1", order: 0 }],
      assignments: {},
    }),
  assertNotContainsB1: (r, msg) =>
    assert.ok(!r.sections.some((s) => s.id === "b1-section"), msg),
  makeA2Edit: (r) => r.createSection("A2-Section"),
  assertA2Derived: (r, msg) =>
    assert.ok(!r.sections.some((s) => s.id === "b1-section"), msg),
  makeLiveRemotePayload: () =>
    JSON.stringify({
      version: 1,
      sections: [{ id: "remote", name: "Remote", order: 0 }],
      assignments: {},
    }),
  assertRemoteNotApplied: (r, msg) =>
    assert.ok(!r.sections.some((s) => s.id === "remote"), msg),
  makeDecryptById: (id) =>
    JSON.stringify({
      version: 1,
      sections: [{ id, name: id, order: 0 }],
      assignments: {},
    }),
  assertLowerIdWon: (r, msg) =>
    assert.deepEqual(
      r.sections.map((s) => s.id),
      ["aaaa"],
      msg,
    ),
  assertHigherIdLost: (r, msg) =>
    assert.ok(!r.sections.some((s) => s.id === "bbbb"), msg),
  makeRemotePayload: () =>
    JSON.stringify({
      version: 1,
      sections: [{ id: "remote", name: "Remote", order: 0 }],
      assignments: {},
    }),
  assertLocalAdoptedAway: (r, msg) =>
    assert.deepEqual(
      r.sections.map((s) => s.id),
      ["remote"],
      msg,
    ),
});

test("assignChannel refreshes an existing assignment before the next eviction", async () => {
  const restoreRelay = stubRelay(relayClient);
  const pubkey = "pk-at-capacity";
  const relayUrl = "wss://relay.example";
  const assignments = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_SECTION_ASSIGNMENTS }, (_, i) => [
      `chan-${String(i).padStart(4, "0")}`,
      "section-1",
    ]),
  );
  window.localStorage.setItem(
    storageKey(pubkey, relayUrl),
    JSON.stringify({
      version: 1,
      sections: [
        { id: "section-1", name: "One", order: 0 },
        { id: "section-2", name: "Two", order: 1 },
      ],
      assignments,
    }),
  );
  try {
    const { result, unmount } = renderHook(() =>
      useChannelSections(pubkey, relayUrl),
    );
    act(() => result.current.assignChannel("chan-0000", "section-2"));
    act(() => result.current.assignChannel("chan-new", "section-1"));
    assert.equal(result.current.assignments["chan-0000"], "section-2");
    assert.equal(result.current.assignments["chan-new"], "section-1");
    assert.equal(result.current.assignments["chan-0001"], undefined);
    assert.equal(
      Object.keys(result.current.assignments).length,
      MAX_CHANNEL_SECTION_ASSIGNMENTS,
    );
    unmount();
  } finally {
    cleanup();
    restoreRelay();
  }
});
// Mutation: writing the consumed marker unconditionally loses the legacy blob.

test("legacy replay whose v2 transfer fails (quota) does not write the consumed marker", async () => {
  const origLocalStorage = window.localStorage;
  const pubkey = "pk-sec-quota";
  const relayUrl = "wss://r.sec-quota";
  const scope = `${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
  const legacyKey = `buzz-channel-sections-outbox.v1:${scope}`;
  const v2Prefix = `buzz-channel-sections-outbox.v1:${scope}:`;
  const legacyRaw = JSON.stringify({
    store: {
      version: 1,
      sections: [{ id: "legacy", name: "Legacy", order: 0 }],
      assignments: {},
    },
    queuedAt: 0,
  });
  const map = new Map([[legacyKey, legacyRaw]]);
  const throwingStorage = {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => {
      if (k.startsWith(v2Prefix)) throw new Error("QuotaExceededError");
      map.set(k, String(v));
    },
    removeItem: (k) => map.delete(k),
    clear: () => map.clear(),
    get length() {
      return map.size;
    },
    key: (i) => [...map.keys()][i] ?? null,
  };
  const restoreRelay = stubRelay(relayClient);
  const restoreTauri = stubTauri(pubkey, null);
  let hook = null;
  try {
    Object.defineProperty(window, "localStorage", {
      value: throwingStorage,
      configurable: true,
    });
    window.localStorage.setItem(
      `buzz-sync-watermark.v1:channel-sections:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`,
      "1700000000",
    );
    await act(async () => {
      hook = renderHook(() => useChannelSections(pubkey, relayUrl));
      for (let i = 0; i < 4; i++) await Promise.resolve();
    });
    assert.equal(
      [...map.keys()].some((k) => k.includes("-legacy-consumed:")),
      false,
      "consumed marker must not be written after failed v2 transfer",
    );
    const resumed = readChannelSectionsOutbox(pubkey, relayUrl);
    assert.ok(resumed !== null, "legacy blob must remain replayable");
    assert.equal(resumed.store.sections[0]?.id, "legacy");
    assert.ok(resumed.legacyRawToConsume !== null);
    hook.unmount();
  } finally {
    cleanup();
    Object.defineProperty(window, "localStorage", {
      value: origLocalStorage,
      configurable: true,
    });
    restoreRelay();
    restoreTauri();
  }
});

runWholeBlobP2aSuite({
  label: "sections",
  Manager: ChannelSectionSyncManager,
  publishEdit: (m, store) => m.publishSections(store),
  subscribe: (m, cb) => m.subscribeToSections(cb),
  makeNonEmptyStore: () => ({
    version: 1,
    sections: [{ id: "s1", name: "Work", order: 0 }],
    assignments: {},
  }),
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeMountStore: () => ({
    version: 1,
    sections: [{ id: "mount", name: "Mount", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
});

runWholeBlobCarlSuite({
  label: "sections",
  outboxKeyPrefix: "buzz-channel-sections-outbox.v1",
  storageKey,
  writeOutboxKey: (pubkey, relayUrl) => {
    const encoded = encodeURIComponent(
      relayUrl.trim().replace(/\/$/, "").toLowerCase(),
    );
    return `buzz-channel-sections-outbox.v1:${pubkey}:${encoded}`;
  },
  readOutbox: readChannelSectionsOutbox,
  useHook: useChannelSections,
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
  assertHookState: (r, label) => {
    assert.deepEqual(
      r.sections,
      [{ id: "remote", name: "Remote", order: 0 }],
      `P*/hook ${label}: hook.sections must reflect adopted remote store — applyRemote returning prev leaves UI stale`,
    );
    assert.deepEqual(
      r.assignments,
      {},
      `P*/hook ${label}: hook.assignments must reflect adopted remote store`,
    );
  },
});

runWholeBlobP2a1Suite({
  label: "sections",
  Manager: ChannelSectionSyncManager,
  publishEdit: (m, store) => m.publishSections(store),
  publishReplay: (m, store) => m.publishSections(store, true),
  subscribe: (m, cb) => m.subscribeToSections(cb),
  makeNonEmptyStore: () => ({
    version: 1,
    sections: [{ id: "s1", name: "Work", order: 0 }],
    assignments: {},
  }),
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
});

runWholeBlobC2Suite({
  label: "sections",
  Manager: ChannelSectionSyncManager,
  publishEdit: (m, store) => m.publishSections(store),
  publishReplay: (m, store) => m.publishSections(store, true),
  subscribe: (m, cb) => m.subscribeToSections(cb),
  makeNonEmptyStore: () => ({
    version: 1,
    sections: [{ id: "s1", name: "Work", order: 0 }],
    assignments: {},
  }),
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
});

runWholeBlobC3Suite({
  label: "sections",
  Manager: ChannelSectionSyncManager,
  publishEdit: (m, store) => m.publishSections(store),
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
});

runWholeBlobP2bSuite({
  label: "sections",
  Manager: ChannelSectionSyncManager,
  publishEdit: (m, store) => m.publishSections(store),
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
});

runWholeBlobP2a1HookSuite({
  label: "sections",
  writeOutboxKey: (pubkey, relayUrl) => {
    const encoded = encodeURIComponent(
      relayUrl.trim().replace(/\/$/, "").toLowerCase(),
    );
    return `buzz-channel-sections-outbox.v1:${pubkey}:${encoded}`;
  },
  storageKey,
  readOutbox: readChannelSectionsOutbox,
  useHook: useChannelSections,
  makeEdit: (r) => r.createSection("P2a1-Section"),
  makeEditStore: () => ({
    version: 1,
    sections: [{ id: "click", name: "Click", order: 0 }],
    assignments: {},
  }),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
  assertHookState: (r, label) => {
    assert.deepEqual(
      r.sections,
      [{ id: "remote", name: "Remote", order: 0 }],
      `P2a-1/hook ${label}: hook.sections must reflect adopted H102 store — applyRemote returning prev leaves UI stale`,
    );
    assert.deepEqual(
      r.assignments,
      {},
      `P2a-1/hook ${label}: hook.assignments must reflect adopted H102 store`,
    );
  },
});

runWholeBlobCarlP2SecondClickSuite({
  label: "sections",
  useHook: useChannelSections,
  makeEdit1: (r) => r.createSection("A1-Section"),
  makeEdit2: (r) => r.createSection("A2-Section"),
  makeRemoteStore: () => ({
    version: 1,
    sections: [{ id: "remote", name: "Remote", order: 0 }],
    assignments: {},
  }),
});
