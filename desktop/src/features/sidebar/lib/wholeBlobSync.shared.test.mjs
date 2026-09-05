// Authoritative whole-blob sync suite — runs against WholeBlobSyncManager via a
// synthetic in-memory config. Lane-specific tests stay in the lane files.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  makeFakeWindow,
  installFakeWindow,
  makeTimerBed,
  installTauriMock,
  installEchoTauri,
  installSeamTauriMock,
  SyntheticWholeBlobManager,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);
const watermarkLane = "channel-sections";

function makeStore(sections = []) {
  return { version: 1, sections, assignments: {} };
}

function nonEmptyStore() {
  return makeStore([{ id: "s1", name: "Work", order: 0 }]);
}
const decryptPayload = JSON.stringify(
  makeStore([{ id: "remote-section-from-relay", name: "Remote", order: 0 }]),
);
const emptyDecryptPayload = JSON.stringify(makeStore([]));

test("destroy: cancels pending publish without flushing; aborts in-flight publish after fetch resolves", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new SyntheticWholeBlobManager("pk-test", RELAY);
    manager.publishSections(nonEmptyStore());
    assert.ok(fw._hasTimer(), "debounce timer should be set");
    manager.destroy();
    assert.ok(!fw._hasTimer(), "debounce timer cleared on destroy");
    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
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
  const fw2 = makeFakeWindow();
  const restore2 = installFakeWindow(fw2);
  try {
    const manager = new SyntheticWholeBlobManager("pk-race", RELAY);
    manager.publishSections(nonEmptyStore());
    fw2._fireTimer();
    manager.destroy();
    releaseFetch();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(publishCalls.length, 0, "must not fire after destroy");
  } finally {
    restore2();
    mock.reset();
  }
});

for (const { title, setupFetch, setupWatermark, pubkey, assertPending } of [
  {
    title: "fetch error does not trigger seed-publish",
    setupFetch: () =>
      mock.method(relayClient, "fetchEvents", () =>
        Promise.reject(new Error("relay timeout")),
      ),
    assertPending: (m) => assert.equal(m.getPendingStore(), null),
  },
  {
    title: "absent fetch with prior watermark blocks seed-publish",
    setupFetch: () =>
      mock.method(relayClient, "fetchEvents", () => Promise.resolve([])),
    setupWatermark: (fw) =>
      fw.localStorage.setItem(
        `buzz-sync-watermark.v1:${watermarkLane}:pk-stale:${RELAY_KEY}`,
        "1700000000",
      ),
    pubkey: "pk-stale",
    assertPending: (m) => assert.equal(m.getPendingStore(), null),
  },
  {
    title: "absent fetch with zero watermark seeds (first-sync preserved)",
    setupFetch: () =>
      mock.method(relayClient, "fetchEvents", () => Promise.resolve([])),
    pubkey: "pk-fresh",
    assertPending: (m) => assert.ok(m.getPendingStore() !== null),
  },
]) {
  test(`revert-fix: ${title}`, async () => {
    setupFetch();
    mock.method(relayClient, "publishEvent", () => Promise.resolve());
    const fw = makeFakeWindow();
    setupWatermark?.(fw);
    const restore = installFakeWindow(fw);
    try {
      const manager = new SyntheticWholeBlobManager(pubkey ?? "pk-fail", RELAY);
      const result = await manager.bootstrap(nonEmptyStore());
      assert.equal(result.action, "hold");
      assertPending(manager);
      manager.destroy();
    } finally {
      restore();
      mock.reset();
    }
  });
}

test("adopt-winner: newer remote adopts and skips publish; local ahead publishes and clears outbox", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-lww",
        content: "good-cipher",
        created_at: 200,
        id: "evt-remote",
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
  const tauri = installTauriMock(decryptPayload);
  try {
    const manager = new SyntheticWholeBlobManager("pk-lww", RELAY);
    const adopted = [];
    manager.setOnRemoteAdopted((r) => adopted.push(r));
    manager.publishSections(nonEmptyStore());
    assert.ok(manager.outboxWrites() > 0, "edit in durable outbox");
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    assert.equal(
      publishCalls.length,
      0,
      "must not publish when remote wins LWW",
    );
    assert.equal(adopted.length, 1, "adopt sink fires");
    assert.ok(
      adopted[0].store.sections.some(
        (s) => s.id === "remote-section-from-relay",
      ),
    );
    assert.equal(manager.getPendingStore(), null);
    assert.equal(manager.outboxClears() >= 1, true, "adopt clears outbox");
    manager.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
  // S1→S2→S1 regression: local edit S1 confirmed, then S2 arrives live while idle,
  // then user re-selects S1. Old code would no-op (lastPublishedStore=S1 still equals pending S1);
  // new code compares against the freshly fetched head (now S2) and publishes.
  // Mutation: storesEqual(S1, S1) no-op at pre-publish check silently drops the re-selection.
  let publishCalls2 = 0;
  let liveCallback2 = null;
  let storedHead2 = [];
  const { fireDelay: fireDelay2, restore: restoreTimers2 } = makeTimerBed();
  const tauri2 = installEchoTauri("pk-s1s2s1-fold");
  mock.method(relayClient, "fetchEvents", () => Promise.resolve(storedHead2));
  mock.method(relayClient, "subscribeLive", (_f, cb) => {
    liveCallback2 = cb;
    return Promise.resolve(async () => {});
  });
  mock.method(relayClient, "publishEvent", (e) => {
    publishCalls2++;
    storedHead2 = [e];
    return Promise.resolve();
  });
  const s1 = makeStore([{ id: "s1", name: "S1", order: 0 }]);
  const s2 = makeStore([{ id: "s2", name: "S2", order: 0 }]);
  try {
    const m = new SyntheticWholeBlobManager("pk-s1s2s1-fold", RELAY);
    // S1 published and confirmed.
    m.publishSections(s1);
    await fireDelay2(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.equal(publishCalls2, 1, "S1 published and confirmed");
    assert.equal(m.getPendingStore(), null, "S1 confirmed — outbox clear");
    // Subscribe; deliver S2 live — storedHead2 advances. Collect the subscription result
    // so we can assert that S2 was actually applied (not just silently ignored).
    const s2Deliveries = [];
    await m.subscribeToSections((r) => s2Deliveries.push(r));
    storedHead2 = [
      tauri2.mintHead(s2, storedHead2[0].created_at + 1, "evt-s2"),
    ];
    liveCallback2(storedHead2[0]);
    for (let i = 0; i < 50; i++) await Promise.resolve();
    // S2 delivery must have fired with the correct sections content.
    assert.equal(
      s2Deliveries.length,
      1,
      "S2 live delivery reached subscription callback",
    );
    assert.ok(
      s2Deliveries[0]?.store?.sections?.some((s) => s.id === "s2"),
      "S2 subscription delivers the s2 sections payload",
    );
    // Re-select S1: pre-publish fetch returns S2; storesEqual(S1,S2)=false → publishes.
    // Old code: storesEqual(S1, lastPublishedStore=S1) = true → no-op.
    m.publishSections(s1);
    // S1 must be pending in the durable outbox BEFORE the timer fires.
    // Removing this confirms the pending state is set before confirmation — the original
    // standalone test's "outboxWrites >= 2 before fire" invariant.
    assert.ok(
      m.getPendingStore() !== null,
      "S1 pending before confirmation timer fires",
    );
    assert.ok(
      m.outboxWrites() >= 2,
      "S1 written to durable outbox before timer fires",
    );
    await fireDelay2(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.equal(
      publishCalls2,
      2,
      "S1 re-published above S2 — stale no-op suppresses this",
    );
    assert.equal(m.getPendingStore(), null, "S1 re-selection confirmed");
    m.destroy();
  } finally {
    tauri2.restore();
    restoreTimers2();
    mock.reset();
  }
});

test("timestamp clamp: published createdAt stays inside the relay future window", async () => {
  const nowSecs = Math.floor(Date.now() / 1000);
  let call = 0;
  mock.method(relayClient, "fetchEvents", () => {
    call++;
    return Promise.resolve([
      {
        pubkey: "pk-clamp",
        content: "good-cipher",
        created_at: call === 1 ? nowSecs + 3_600 : 0,
        id: "evt-clamp",
      },
    ]);
  });
  let signedCreatedAt = null;
  mock.method(relayClient, "publishEvent", (evt) => {
    signedCreatedAt = evt.created_at;
    return Promise.resolve();
  });
  const fw = makeFakeWindow(),
    restore = installFakeWindow(fw),
    tauri = installTauriMock(emptyDecryptPayload);
  try {
    const manager = new SyntheticWholeBlobManager("pk-clamp", RELAY);
    await manager.fetchRemoteSections();
    manager.publishSections(nonEmptyStore());
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(signedCreatedAt !== null, "publish must have been attempted");
    assert.ok(
      signedCreatedAt <= Math.floor(Date.now() / 1000) + 840,
      `createdAt clamped — got ${signedCreatedAt}`,
    );
    manager.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("unreadable head (decrypt failure) retains the pending edit and retries, never publishing", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      { pubkey: "pk-undec", content: "bad-cipher", created_at: 500, id: "evt" },
    ]),
  );
  const publishCalls = [];
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow(),
    restore = installFakeWindow(fw),
    t = installTauriMock("{}");
  try {
    const manager = new SyntheticWholeBlobManager("pk-undec", RELAY);
    manager.publishSections(nonEmptyStore());
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    assert.equal(publishCalls.length, 0, "must not publish");
    assert.ok(manager.getPendingStore() !== null, "pending edit retained");
    assert.ok(manager.outboxWrites() > 0, "pending edit in durable outbox");
    assert.ok(fw._hasTimer(), "retry scheduled");
    manager.destroy();
  } finally {
    t.restore();
    restore();
    mock.reset();
  }
});

test("live remote during debounce is adopted at pre-publish, not overwritten", async () => {
  let liveCallback = null;
  mock.method(relayClient, "subscribeLive", (_f, onEvent) => {
    liveCallback = onEvent;
    return Promise.resolve(async () => {});
  });
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-live-deb",
        content: "good-cipher",
        created_at: 500,
        id: "evt-live",
      },
    ]),
  );
  const publishCalls = [];
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const { fireDelay, restore } = makeTimerBed();
  const tauri = installTauriMock(
    JSON.stringify(
      makeStore([{ id: "remote-during-debounce", name: "Remote", order: 0 }]),
    ),
  );
  try {
    const manager = new SyntheticWholeBlobManager("pk-live-deb", RELAY);
    const adopted = [];
    manager.setOnRemoteAdopted((r) => adopted.push(r));
    await manager.subscribeToSections(() => {});
    manager.publishSections(
      makeStore([{ id: "local", name: "Local", order: 0 }]),
    );
    liveCallback({
      pubkey: "pk-live-deb",
      content: "good-cipher",
      created_at: 500,
      id: "evt-live",
    });
    for (let i = 0; i < 50; i++) await Promise.resolve();
    await fireDelay(2000);
    assert.equal(publishCalls.length, 0, "must not publish during debounce");
    assert.equal(adopted.length, 1, "newer remote must be adopted");
    manager.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("same-second collision: second-edit queued during confirmation survives; loser adopts and retries", async () => {
  {
    let fetchCalls = 0,
      releaseConfirmation = null,
      publishCalls = 0,
      storedHead = [];
    const { fireDelay, restore } = makeTimerBed();
    const tauri = installEchoTauri("pk-collide2");
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      if (fetchCalls === 1) return Promise.resolve([]);
      if (fetchCalls === 2)
        return new Promise((res) => {
          releaseConfirmation = () => res(storedHead);
        });
      return Promise.resolve(storedHead);
    });
    const winnerStore = makeStore([{ id: "peer", name: "Peer", order: 0 }]);
    mock.method(relayClient, "publishEvent", (event) => {
      publishCalls++;
      if (publishCalls === 1) {
        storedHead = [
          tauri.mintHead(winnerStore, event.created_at, "0-peer-winner"),
        ];
        return Promise.resolve();
      }
      storedHead = [event];
      return Promise.resolve();
    });
    try {
      const manager = new SyntheticWholeBlobManager("pk-collide2", RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));
      manager.publishSections(
        makeStore([{ id: "mine", name: "Mine", order: 0 }]),
      );
      await fireDelay(2000);
      while (releaseConfirmation === null) await Promise.resolve();
      manager.publishSections(
        makeStore([{ id: "second", name: "Second", order: 0 }]),
      );
      releaseConfirmation();
      for (let i = 0; i < 100; i++) await Promise.resolve();
      assert.equal(
        adopted.length,
        0,
        "stale-gen adopt must not fire onRemoteAdopted for the newer pending edit",
      );
      assert.notEqual(
        manager.getPendingStore(),
        null,
        "edit-2 still pending after stale-gen adopt",
      );
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();
      assert.equal(publishCalls, 2, "edit-2 publishes above the peer winner");
      assert.equal(adopted.length, 0, "edit-2 must not be adopted away");
      assert.equal(
        manager.getPendingStore(),
        null,
        "edit-2 clears via confirmed publish",
      );
      assert.equal(manager.outboxClears() >= 1, true, "edit-2 outbox cleared");
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  }
  {
    let fetchCalls = 0,
      publishCalls = 0,
      storedHead = [];
    const { fireDelay, restore } = makeTimerBed();
    const tauri = installEchoTauri("pk-loser");
    const winnerStore = makeStore([{ id: "peer", name: "Peer", order: 0 }]);
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      return Promise.resolve(fetchCalls === 1 ? [] : storedHead);
    });
    mock.method(relayClient, "publishEvent", (event) => {
      publishCalls++;
      if (publishCalls === 1)
        storedHead = [
          tauri.mintHead(winnerStore, event.created_at, "0-peer-winner"),
        ];
      return Promise.resolve();
    });
    try {
      const manager = new SyntheticWholeBlobManager("pk-loser", RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));
      manager.publishSections(
        makeStore([{ id: "loser", name: "Loser", order: 0 }]),
      );
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();
      assert.equal(adopted.length, 1, "loser must adopt the peer winner");
      assert.equal(
        manager.getPendingStore(),
        null,
        "pending cleared after adopt",
      );
      manager.publishSections(
        makeStore([{ id: "mine", name: "Mine", order: 0 }]),
      );
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();
      assert.equal(
        publishCalls,
        2,
        "edit after adopt must publish successfully",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  }
});

test("revert-fix: undecryptable live event advances watermark before decrypt attempt", async () => {
  let liveCallback = null;
  mock.method(relayClient, "subscribeLive", (_filter, onEvent) => {
    liveCallback = onEvent;
    return Promise.resolve(async () => {});
  });
  const fw = makeFakeWindow(),
    restore = installFakeWindow(fw);
  try {
    const manager = new SyntheticWholeBlobManager("pk-live", RELAY);
    const wmKey = `buzz-sync-watermark.v1:${watermarkLane}:pk-live:${RELAY_KEY}`;
    assert.equal(
      fw.localStorage.getItem(wmKey),
      null,
      "watermark starts absent",
    );
    await manager.subscribeToSections(() => {});
    assert.ok(liveCallback !== null, "subscribeLive captured the callback");
    liveCallback({
      pubkey: "pk-live",
      content: "!bad-cipher!",
      created_at: 1700005555,
      id: "live-evt-1",
    });
    await new Promise((r) => setTimeout(r, 0));
    assert.ok(Number(fw.localStorage.getItem(wmKey) ?? "0") >= 1700005555);
    manager.destroy();
  } finally {
    restore();
    mock.reset();
  }
});

const SEAM_PAYLOAD = JSON.stringify({
  version: 1,
  sections: [{ id: "a", name: "A", order: 0 }],
  assignments: {},
});
const SEAM_EVENT = (id) => ({
  id,
  pubkey: "pk-ambiguous",
  content: "good-cipher",
  created_at: 500,
  kind: 30078,
  tags: [["d", "channel-sections"]],
  sig: "s",
});

// Mutation: dropping ambiguousAttemptIds fold makes B classify A as foreign and adopt.
test("ambiguous ACK: an accepted-but-unacked A does not make B adopt and disappear", async () => {
  let releaseFirst = null,
    publishCalls = 0,
    storedHead = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve(storedHead));
  mock.method(relayClient, "publishEvent", (event) => {
    publishCalls++;
    if (publishCalls === 1)
      return new Promise((_res, reject) => {
        releaseFirst = () => {
          storedHead = [SEAM_EVENT("event-a")];
          reject(new Error("Timed out publishing channel sections."));
        };
      });
    storedHead = [SEAM_EVENT("event-b")];
    return Promise.resolve();
  });
  const { timers, fireDelay, restore } = makeTimerBed();
  const tauri = installSeamTauriMock(
    SEAM_PAYLOAD,
    ["event-a", "event-b"],
    "pk-ambiguous",
  );
  try {
    const manager = new SyntheticWholeBlobManager("pk-ambiguous", RELAY);
    const adopted = [];
    manager.setOnRemoteAdopted((r) => adopted.push(r.eventId));
    manager.publishSections(makeStore([{ id: "a", name: "A", order: 0 }]));
    await fireDelay(2000);
    while (releaseFirst === null) await Promise.resolve();
    manager.publishSections(makeStore([{ id: "b", name: "B", order: 0 }]));
    releaseFirst();
    for (let i = 0; i < 100; i++) await Promise.resolve();
    if ([...timers.values()].some((t) => t.ms === 2000)) await fireDelay(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.deepEqual(adopted, [], "B must not adopt A when A's ACK was lost");
    assert.equal(
      publishCalls,
      2,
      "B publishes above A's ambiguously-accepted head",
    );
    assert.equal(manager.getPendingStore(), null);
    manager.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

// Mutation: dropping the id-guard (folding any advance) makes B erase the foreign winner.
test("ambiguous ACK: a foreign head is adopted, not folded as our own", async () => {
  let publishCalls = 0,
    fetchCalls = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetchCalls++;
    if (fetchCalls === 1) return Promise.resolve([]);
    return Promise.resolve([
      {
        id: "foreign-winner",
        pubkey: "pk-reject",
        content: "good-cipher",
        created_at: 500,
        kind: 30078,
        tags: [["d", "channel-sections"]],
        sig: "s",
      },
    ]);
  });
  mock.method(relayClient, "publishEvent", () => {
    publishCalls++;
    if (publishCalls === 1)
      return Promise.reject(
        new Error("Timed out publishing channel sections."),
      );
    return Promise.resolve();
  });
  const { timers, fireDelay, restore } = makeTimerBed();
  const tauri = installSeamTauriMock(
    SEAM_PAYLOAD,
    ["event-a", "event-b"],
    "pk-reject",
  );
  try {
    const manager = new SyntheticWholeBlobManager("pk-reject", RELAY);
    const adopted = [];
    manager.setOnRemoteAdopted((r) => adopted.push(r.eventId));
    manager.publishSections(makeStore([{ id: "a", name: "A", order: 0 }]));
    await fireDelay(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    manager.publishSections(makeStore([{ id: "b", name: "B", order: 0 }]));
    if ([...timers.values()].some((t) => t.ms === 2000)) await fireDelay(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.deepEqual(adopted, ["foreign-winner"], "B adopts the foreign head");
    assert.equal(manager.getPendingStore(), null, "adopt clears pending");
    manager.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

// Mutation: removing the confirmed prior-gen baseline fold (wholeBlobSyncManager.ts line 721)
// makes edit-2 adopt edit-1's confirmed head instead of publishing above it.
test("prior-gen fold: edit-2 queued during e1 publish still publishes above e1's confirmed head", async () => {
  let releasePublish = null,
    publishCalls = 0,
    storedHead = [];
  const { fireDelay, restore } = makeTimerBed();
  const tauri = installEchoTauri("pk-pg2");
  let blockPublish = true;
  mock.method(relayClient, "fetchEvents", () => Promise.resolve(storedHead));
  mock.method(relayClient, "publishEvent", (e) => {
    publishCalls++;
    storedHead = [e];
    if (blockPublish) {
      blockPublish = false;
      return new Promise((res) => {
        releasePublish = () => res();
      });
    }
    return Promise.resolve();
  });
  try {
    const m = new SyntheticWholeBlobManager("pk-pg2", RELAY);
    const adopted = [];
    m.setOnRemoteAdopted((r) => adopted.push(r));
    m.publishSections(makeStore([{ id: "e1", name: "E1", order: 0 }]));
    await fireDelay(2000);
    while (!releasePublish) await Promise.resolve();
    m.publishSections(
      makeStore([
        { id: "e2a", name: "A", order: 0 },
        { id: "e2b", name: "B", order: 1 },
      ]),
    );
    releasePublish();
    for (let i = 0; i < 200; i++) await Promise.resolve();
    assert.ok(adopted.length === 0, "edit-2 must not adopt e1's head");
    assert.notEqual(m.getPendingStore(), null, "edit-2 still pending");
    await fireDelay(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.equal(publishCalls, 2, "edit-2 publishes above e1's head");
    assert.equal(m.getPendingStore(), null);
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

// Mutation: removing the queuedAt guard lets a stale outbox replay above the head.
test("stale outbox replay (P1): bootstrap records lastRemoteHead; post-bootstrap publish fires above the head", async () => {
  // This manager-level test confirms bootstrap records the fetched head so that
  // a publish() immediately after freezes publishBaseline to the real head and
  // fires correctly. The hook-level queuedAt guard is exercised in wholeBlobHook.shared.test.mjs.
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-stale-replay",
        content: "good-cipher",
        created_at: 200,
        id: "evt-head",
      },
    ]),
  );
  const publishCalls = [];
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const { fireDelay, restore } = makeTimerBed();
  const tauri = installTauriMock(
    JSON.stringify(
      makeStore([{ id: "remote-from-relay", name: "Remote", order: 0 }]),
    ),
  );
  try {
    const manager = new SyntheticWholeBlobManager("pk-stale-replay", RELAY);
    await manager.bootstrap(nonEmptyStore());
    assert.equal(
      manager.getPendingStore(),
      null,
      "no pending edit after bootstrap",
    );
    // An edit queued after bootstrap should fire: baseline is {200,"evt-head"},
    // pre-publish fetch returns {200,"evt-head"} → remoteAdvancedSince = false → publish.
    manager.publishSections(
      makeStore([{ id: "fresh-edit", name: "Fresh", order: 0 }]),
    );
    await fireDelay(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.equal(
      publishCalls.length,
      1,
      "edit queued after bootstrap publishes above the head",
    );
    manager.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

// Mutation: reverting to synchronous recordRemoteHead in subscribeLive re-opens P2b.
test("P2b decrypt gap: watermark advances synchronously; lastRemoteHead advances only after successful decrypt", async () => {
  // After a live event at createdAt=300 that fails to decrypt, a subsequent
  // publish() must NOT freeze publishBaseline to {300,"live-evt"}: the head tuple
  // is unconfirmed until decrypt succeeds. When the pre-publish fetch returns the
  // same event as decryptable, it is a genuine advance and triggers adopt rather
  // than a silent publish over the unseen remote-only changes (Kalvin P2b).
  let liveCallback = null;
  mock.method(relayClient, "subscribeLive", (_f, cb) => {
    liveCallback = cb;
    return Promise.resolve(async () => {});
  });
  // Pre-publish fetch returns the same live event, now treated as decryptable
  // by the Tauri mock (good-cipher). fetchOwnBlobBeforePublish sees the 300-head,
  // remoteAdvancedSince({300,"live-evt"},{0,""}) = true → adopt; no publish.
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-p2b",
        content: "good-cipher",
        created_at: 300,
        id: "live-evt",
      },
    ]),
  );
  const publishCalls = [];
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const { fireDelay, restore } = makeTimerBed();
  const remotePayload = JSON.stringify(
    makeStore([{ id: "remote-live", name: "Remote", order: 0 }]),
  );
  const tauri = installTauriMock(remotePayload);
  const fw = makeFakeWindow();
  const wmKey = `buzz-sync-watermark.v1:${watermarkLane}:pk-p2b:${RELAY_KEY}`;
  const origLs = globalThis.window?.localStorage;
  try {
    if (typeof globalThis.window === "undefined") globalThis.window = {};
    globalThis.window.localStorage = fw.localStorage;
    const manager = new SyntheticWholeBlobManager("pk-p2b", RELAY);
    await manager.subscribeToSections(() => {});
    assert.ok(liveCallback !== null, "live subscription installed");
    // Deliver a live event whose content will fail decrypt (bad-cipher).
    // Watermark must advance synchronously; lastRemoteHead must NOT advance
    // (head tuple unconfirmed until decrypt succeeds).
    liveCallback({
      pubkey: "pk-p2b",
      content: "bad-cipher",
      created_at: 300,
      id: "live-evt",
    });
    assert.ok(
      Number(fw.localStorage.getItem(wmKey) ?? "0") >= 300,
      "watermark advances synchronously for undecryptable live event",
    );
    // With fix: lastRemoteHead = {0,""} → publishBaseline frozen to {0,""}.
    // Pre-publish fetch returns {300,"live-evt"} decryptable → advance → adopt.
    // Without fix: baseline would be {300,"live-evt"} → no advance → publish
    // (pre-live content silently overwrites unseen remote state).
    const adopted = [];
    manager.setOnRemoteAdopted((r) => adopted.push(r));
    manager.publishSections(
      makeStore([{ id: "pre-live", name: "Pre", order: 0 }]),
    );
    await fireDelay(2000);
    for (let i = 0; i < 100; i++) await Promise.resolve();
    assert.equal(
      publishCalls.length,
      0,
      "pre-live edit must not publish over unseen remote",
    );
    assert.equal(
      adopted.length,
      1,
      "edit adopted away — remote is the true head",
    );
    manager.destroy();
  } finally {
    if (origLs !== undefined) globalThis.window.localStorage = origLs;
    tauri.restore();
    restore();
    mock.reset();
  }
});
