import assert from "node:assert/strict";
import test, { beforeEach, mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { SECTIONS_LANE, projectSections } from "./channelSectionsSync.ts";
import { SORT_LANE, projectSort } from "./channelSortSync.ts";
import {
  PUBLISH_CANCELED,
  publishSessionEvent,
} from "@/shared/api/relayEventPublisher";
import { LaneReconciler } from "./sidebarLaneReconciler.ts";
import { LaneStore } from "./sidebarLaneStore.ts";
import { canonical, setRegs } from "./sidebarLwwMap.ts";

const PK = "f".repeat(64);
const RELAY = "wss://r.test";
const storage = new Map();
globalThis.window ??= {};
Object.assign(globalThis.window, {
  localStorage: {
    getItem: (k) => storage.get(k) ?? null,
    setItem: (k, v) => {
      if (fx.storageFails) throw new Error("quota");
      fx.writes++;
      storage.set(k, v);
    },
    removeItem: (k) => storage.delete(k),
  },
  addEventListener: () => {},
  removeEventListener: () => {},
});

/** Fake relay (one replaceable head), identity crypto, scripted failures. */
let fx;
let seq = 0;
beforeEach(() => {
  storage.clear();
  mock.restoreAll();
  fx = {
    head: null,
    published: [],
    fetchFails: false,
    publish: "ok",
    storageFails: false,
    writes: 0,
    badDecrypt: new Set(),
  };
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd === "nip44_encrypt_to_self") return args.plaintext;
      if (cmd === "nip44_decrypt_from_self") {
        if (fx.badDecrypt.has(args.ciphertext)) throw new Error("bad");
        if (fx.holdDecrypt && --fx.holdAfter < 0) await fx.holdDecrypt;
        return args.ciphertext;
      }
      if (cmd === "sign_event")
        return JSON.stringify(ev(args.content, args.createdAt));
      throw new Error(`unmocked ${cmd}`);
    },
  };
  mock.method(relayClient, "fetchEvents", async () => {
    const result = fx.head ? [fx.head] : []; // snapshot at request time
    if (fx.gate) await fx.gate;
    if (fx.fetchFails) throw new Error("offline");
    return result;
  });
  mock.method(console, "warn", () => {});
  mock.method(relayClient, "publishEvent", async (event, _t, _e, isCurrent) => {
    fx.beforeSend?.();
    if (!isCurrent()) throw new Error(PUBLISH_CANCELED);
    fx.published.push(event);
    if (fx.publish === "timeout") throw new Error("Timed out");
    if (fx.publish === "reject") throw new Error("blocked: nope");
    if (fx.publish === "duplicate") throw new Error("duplicate: have it");
    if (fx.publish !== "lost") fx.head = event;
  });
});

function ev(content, createdAt) {
  seq++;
  return {
    id: String(seq).padStart(64, "0"),
    pubkey: PK,
    kind: 30078,
    created_at: createdAt,
    content,
    tags: [],
    sig: "s",
  };
}

function device(lane) {
  const store = new LaneStore(lane, PK, RELAY);
  const rec = new LaneReconciler(lane, store, PK, RELAY);
  rec.wake = () => {}; // timers are driven explicitly via read()
  return { store, rec };
}

const settle = () => new Promise((r) => setImmediate(r));
async function sync(d) {
  await d.rec.read();
  for (let i = 0; i < 5; i++) await settle();
}

const LANES = [
  {
    name: "sections",
    lane: SECTIONS_LANE,
    edit: (tree, key, value) =>
      setRegs(tree, [
        [["s", key, "name"], value],
        [["s", key, "order"], 0],
        [["s", key, "live"], true],
      ]),
    remove: (tree, key) => setRegs(tree, [[["s", key, "live"], false]]),
    view: (tree) =>
      Object.fromEntries(
        projectSections(tree).sections.map((s) => [s.id, s.name]),
      ),
    legacy: (items) => ({
      version: 1,
      sections: Object.entries(items).map(([id, name], order) => ({
        id,
        name,
        order,
      })),
      assignments: {},
    }),
    big: (tree) =>
      setRegs(
        tree,
        Array.from({ length: 101 }, (_, i) => [
          ["s", `s${i}`, "live"],
          true,
        ]).flatMap((w, i) => [
          w,
          [["s", `s${i}`, "name"], "n"],
          [["s", `s${i}`, "order"], i],
        ]),
      ),
  },
  {
    name: "sort",
    lane: SORT_LANE,
    edit: (tree, key, value) =>
      setRegs(tree, [
        [
          ["g", `section:${key}`],
          value === "B" || value === "Y" ? "recent" : "alpha",
        ],
      ]),
    remove: (tree, key) => setRegs(tree, [[["g", `section:${key}`], null]]),
    view: (tree) =>
      Object.fromEntries(
        Object.entries(projectSort(tree).groups).map(([k, m]) => [
          k.slice(8),
          m === "recent" ? "B" : "A",
        ]),
      ),
    legacy: (items) => ({
      version: 1,
      groups: Object.fromEntries(
        Object.entries(items).map(([k, v]) => [
          `section:${k}`,
          v === "B" ? "recent" : "alpha",
        ]),
      ),
    }),
    big: (tree) =>
      setRegs(
        tree,
        Array.from({ length: 105 }, (_, i) => [
          ["g", `section:${i}`],
          "recent",
        ]),
      ),
  },
];

for (const L of LANES) {
  test(`${L.name}: new scope with local data publishes its first copy`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    await sync(d);
    assert.equal(fx.published.length, 1);
    const doc = JSON.parse(fx.published[0].content);
    assert.equal(doc.version, 1);
    assert.equal(doc.meta.v, 1);
    await sync(d);
    assert.equal(fx.published.length, 1, "equal digest: no republish");
  });

  test(`${L.name}: absence after a seen head holds (never publishes over it)`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    await sync(d);
    fx.head = null; // relay transiently returns nothing
    const d2 = device(L.lane); // restart: watermark > 0
    d2.store.transact((t) => L.edit(t, "k2", "B"));
    await sync(d2);
    assert.equal(fx.published.length, 1);
    assert.equal(d2.rec.head.status, "unknown");
  });

  test(`${L.name}: stale device merges instead of overwriting newer items`, async () => {
    const a = device(L.lane);
    a.store.transact((t) => L.edit(t, "k1", "A"));
    await sync(a);
    storage.clear(); // B: separate install with its own local cache
    const b = device(L.lane);
    b.store.transact((t) => L.edit(t, "k2", "B"));
    await sync(b);
    assert.deepEqual(L.view(JSON.parse(fx.head.content).meta), {
      k1: "A",
      k2: "B",
    });
    await sync(a);
    assert.deepEqual(L.view(a.store.get()), { k1: "A", k2: "B" });
  });

  test(`${L.name}: a delete on one device survives a stale device's publish`, async () => {
    const a = device(L.lane);
    a.store.transact((t) => L.edit(L.edit(t, "k1", "A"), "k2", "B"));
    await sync(a);
    const snapshot = new Map(storage);
    a.store.transact((t) => L.remove(t, "k1"));
    await sync(a);
    storage.clear();
    for (const [k, v] of snapshot) if (!k.includes("clock")) storage.set(k, v);
    const stale = device(L.lane); // cache from before the delete
    stale.store.transact((t) => L.edit(t, "k3", "A"));
    await sync(stale);
    assert.deepEqual(L.view(JSON.parse(fx.head.content).meta), {
      k2: "B",
      k3: "A",
    });
  });

  test(`${L.name}: old writer without meta adds items but cannot delete`, async () => {
    const a = device(L.lane);
    a.store.transact((t) => L.edit(t, "k1", "A"));
    await sync(a);
    fx.head = ev(JSON.stringify(L.legacy({ k9: "B" })), fx.head.created_at + 1);
    await sync(a);
    assert.deepEqual(L.view(a.store.get()), { k1: "A", k9: "B" });
    assert.equal(
      JSON.parse(fx.head.content).meta.v,
      1,
      "next upgraded publish restores meta",
    );
  });

  test(`${L.name}: meta-less local cache imports once at a stable stamp`, async () => {
    storage.set(
      L.lane.storageKey(PK, RELAY),
      JSON.stringify(L.legacy({ k1: "A" })),
    );
    const d = device(L.lane);
    const first = canonical(d.store.get());
    assert.deepEqual(L.view(d.store.get()), { k1: "A" });
    assert.equal(canonical(device(L.lane).store.get()), first);
  });

  test(`${L.name}: publish exits (timeout, reject, duplicate) release the attempt`, async (t) => {
    t.mock.timers.enable({ apis: ["Date"], now: 1e12 });
    for (const outcome of ["timeout", "reject", "duplicate"]) {
      storage.clear();
      fx.head = null;
      fx.publish = outcome;
      const d = device(L.lane);
      d.store.transact((t) => L.edit(t, "k1", "A"));
      const before = fx.published.length;
      await sync(d);
      assert.equal(fx.published.length, before + 1, outcome);
      fx.publish = "ok";
      if (outcome === "duplicate") fx.head = fx.published.at(-1);
      t.mock.timers.tick(60_000); // past the failure backoff
      await sync(d);
      assert.equal(
        fx.published.length,
        before + (outcome === "duplicate" ? 1 : 2),
        `${outcome} retry`,
      );
    }
  });

  test(`${L.name}: preflight failure inside an attempt backs off, then recovers`, async (t) => {
    t.mock.timers.enable({ apis: ["Date"], now: 1e12 });
    const d = device(L.lane);
    d.store.transact((tr) => L.edit(tr, "k2", "B"));
    fx.fetchFails = true; // the attempt's own preflight read fails
    await d.rec.ingest(ev(JSON.stringify(L.legacy({ k1: "A" })), 100));
    for (let i = 0; i < 5; i++) await settle();
    assert.equal(fx.published.length, 0);
    fx.fetchFails = false;
    fx.head = ev(JSON.stringify(L.legacy({ k1: "A" })), 100);
    await sync(d);
    assert.equal(fx.published.length, 0, "held by the backoff");
    t.mock.timers.tick(5_000);
    await sync(d);
    assert.equal(fx.published.length, 1);
  });

  test(`${L.name}: a stale preflight decode cannot undo a newer absence`, async () => {
    const h0 = ev(JSON.stringify(L.legacy({ k1: "A" })), 100);
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k2", "B"));
    let release;
    fx.holdDecrypt = new Promise((r) => (release = r));
    fx.holdAfter = 1; // the attempt's preflight decode of H0 waits
    fx.head = h0;
    void d.rec.ingest(h0);
    for (let i = 0; i < 5; i++) await settle();
    fx.head = null;
    await d.rec.read(); // recovery read during the acquired attempt
    release();
    for (let i = 0; i < 5; i++) await settle();
    assert.equal(fx.published.length, 0);
    assert.equal(d.rec.head.status, "unknown");
    fx.holdDecrypt = null;
    fx.head = h0;
    await sync(d);
    assert.equal(fx.published.length, 1, "a later readable head re-enables");
  });

  for (const fails of [false, true]) {
    const what = fails ? "a failure to its full backoff" : "the send";
    test(`${L.name}: an edit during preflight holds ${what}`, async (t) => {
      t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1e12 });
      const h0 = ev(JSON.stringify(L.legacy({ k1: "A" })), 100);
      const store = new LaneStore(L.lane, PK, RELAY);
      const rec = new LaneReconciler(L.lane, store, PK, RELAY);
      store.transact((tr) => L.edit(tr, "k2", "B"));
      let open;
      fx.gate = new Promise((r) => (open = r));
      fx.head = h0;
      void rec.ingest(h0); // attempt acquired; preflight fetch waits
      for (let i = 0; i < 5; i++) await settle();
      store.transact((tr) => L.edit(tr, "k3", "A"));
      rec.defer(); // edit deadline: +2 s
      fx.fetchFails = fails; // the acquired preflight then really fails
      fx.gate = null;
      open();
      for (let i = 0; i < 5; i++) await settle();
      fx.fetchFails = false;
      assert.equal(fx.published.length, 0);
      t.mock.timers.tick(2_000); // real wake() re-arms; no manual read
      for (let i = 0; i < 5; i++) await settle();
      assert.equal(fx.published.length, fails ? 0 : 1, "edit deadline");
      t.mock.timers.tick(3_000);
      for (let i = 0; i < 5; i++) await settle();
      assert.equal(fx.published.length, 1, "after the 5 s failure backoff");
      rec.destroy();
    });
  }

  test(`${L.name}: unreadable head holds quietly; content never published over it`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    fx.head = ev("not json", 10);
    await sync(d);
    assert.equal(d.rec.head.status, "unreadable");
    assert.equal(fx.published.length, 0);
  });

  test(`${L.name}: stale decode merges content but does not settle status`, async () => {
    const d = device(L.lane);
    const older = ev(JSON.stringify(L.legacy({ k1: "A" })), 10);
    const newer = ev("garbage", 11);
    fx.badDecrypt.add("garbage");
    const p = d.rec.ingest(older);
    await d.rec.ingest(newer); // newer head observed before older decode lands
    await p;
    assert.equal(d.rec.head.id, newer.id);
    assert.equal(d.rec.head.status, "unreadable");
    assert.deepEqual(L.view(d.store.get()), { k1: "A" });
  });

  test(`${L.name}: over limit stays local and is not published`, async () => {
    const d = device(L.lane);
    d.store.transact(L.big);
    await sync(d);
    assert.equal(fx.published.length, 0);
    assert.ok(
      storage.get(L.lane.storageKey(PK, RELAY)).length > 1000,
      "kept durable locally",
    );
  });

  test(`${L.name}: destroyed reconciler never publishes`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    d.rec.destroy();
    await sync(d);
    assert.equal(fx.published.length, 0);
  });
}

test("sections: projection is live-only, dense integer order, live assignments", () => {
  const A = "a".repeat(16);
  const tree = {
    s: {
      x: { name: [1, A, "X"], order: [1, A, 7], live: [1, A, true] },
      y: { name: [1, A, "Y"], order: [1, A, 3], live: [1, A, true] },
      z: { name: [1, A, "Z"], order: [1, A, 0], live: [1, A, false] },
    },
    a: { c1: [1, A, "x"], c2: [1, A, "z"], c3: [1, A, null] },
  };
  assert.deepEqual(projectSections(tree), {
    sections: [
      { id: "y", name: "Y", order: 0 },
      { id: "x", name: "X", order: 1 },
    ],
    assignments: { c1: "x" },
  });
});

test("sections: rename concurrent with delete stays deleted", async () => {
  const d = device(SECTIONS_LANE);
  d.store.transact((t) => LANES[0].edit(t, "k1", "A"));
  const base = d.store.get();
  const renamed = setRegs(base, [[["s", "k1", "name"], "B"]]);
  const deleted = setRegs(base, [[["s", "k1", "live"], false]]);
  d.store.merge(renamed);
  d.store.merge(deleted);
  assert.deepEqual(projectSections(d.store.get()).sections, []);
});

// ─── pass-1 regressions ────────────────────────────────────────────────────

for (const L of LANES) {
  test(`${L.name}: absence on a running reconciler holds until a head returns`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    await sync(d);
    const h1 = fx.head;
    fx.head = null; // transient empty read after H0 was decoded
    d.store.transact((t) => L.edit(t, "k2", "B"));
    await sync(d);
    assert.equal(fx.published.length, 1);
    assert.equal(d.rec.head.status, "unknown");
    fx.head = h1;
    await sync(d);
    assert.equal(fx.published.length, 2, "readable head re-enables publish");
  });

  test(`${L.name}: empty verification after an ACK does not republish`, async () => {
    fx.publish = "lost"; // ACKed, but reads keep returning nothing
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    await sync(d);
    await sync(d);
    await sync(d);
    assert.equal(fx.published.length, 1);
  });

  test(`${L.name}: a stale absent read cannot demote a newer observation`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    let open;
    fx.gate = new Promise((r) => (open = r));
    const pending = d.rec.read(); // absent, still in flight
    fx.gate = null;
    fx.head = ev(JSON.stringify(L.legacy({ k1: "A" })), 50);
    await d.rec.ingest(fx.head);
    open();
    await pending;
    assert.equal(d.rec.head.status, "decoded");
    await sync(d); // drain the attempt this head enabled
  });

  test(`${L.name}: an imported empty container settles without publishing`, async () => {
    storage.set(L.lane.storageKey(PK, RELAY), JSON.stringify(L.legacy({})));
    const d = device(L.lane);
    await sync(d);
    assert.equal(fx.published.length, 0);
  });

  test(`${L.name}: backoff holds recovery, live and reconnect attempts`, async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1e12 });
    const store = new LaneStore(L.lane, PK, RELAY);
    const rec = new LaneReconciler(L.lane, store, PK, RELAY);
    fx.publish = "reject";
    store.transact((tr) => L.edit(tr, "k1", "A"));
    await rec.read();
    for (let i = 0; i < 5; i++) await settle();
    assert.equal(fx.published.length, 1);
    fx.publish = "ok";
    await rec.read(); // recovery tick at t+0
    rec.wake(); // reconnect must not shorten the 5 s backoff
    t.mock.timers.tick(4_000);
    for (let i = 0; i < 5; i++) await settle();
    assert.equal(fx.published.length, 1, "held inside backoff");
    t.mock.timers.tick(1_000);
    for (let i = 0; i < 5; i++) await settle();
    assert.equal(fx.published.length, 2, "published after the deadline");
    rec.destroy();
  });

  test(`${L.name}: a change during the publish wait cancels the send`, async () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    fx.beforeSend = () => d.store.transact((t) => L.edit(t, "k2", "B"));
    await sync(d);
    assert.equal(fx.published.length, 0);
    fx.beforeSend = null;
    d.rec.destroy();
  });

  test(`${L.name}: legacy pubkey key migrates once and stays relay-scoped`, () => {
    const legacyKey = L.lane.legacyStorageKey?.(PK);
    if (!legacyKey) return;
    storage.set(legacyKey, JSON.stringify(L.legacy({ k1: "A" })));
    fx.storageFails = true;
    const failed = new LaneStore(L.lane, PK, RELAY);
    assert.deepEqual(L.view(failed.get()), { k1: "A" });
    assert.ok(storage.has(legacyKey), "kept until the scoped write lands");
    fx.storageFails = false;
    failed.persist(); // same store, e.g. the next recovery tick
    assert.ok(!storage.has(legacyKey));
    const other = new LaneStore(L.lane, PK, "wss://other.test");
    assert.deepEqual(L.view(other.get()), {}, "second relay starts clean");
    storage.set(legacyKey, JSON.stringify(L.legacy({ k9: "B" })));
    assert.deepEqual(L.view(new LaneStore(L.lane, PK, RELAY).get()), {
      k1: "A",
    });
  });

  test(`${L.name}: two tabs converge and go quiet`, () => {
    const handlers = [];
    window.addEventListener = (_type, h) => handlers.push(h);
    const tabs = [0, 1].map(() => new LaneStore(L.lane, PK, RELAY));
    for (const tab of tabs) tab.attachCrossTab();
    window.addEventListener = () => {};
    const key = L.lane.storageKey(PK, RELAY);
    const deliver = (to) =>
      handlers[to]({ key, newValue: storage.get(key) ?? null });
    tabs[0].transact((t) => L.edit(t, "k1", "A"));
    const lost = storage.get(key);
    tabs[1].transact((t) => L.edit(t, "k2", "B")); // overwrites tab 0's write
    handlers[1]({ key, newValue: lost });
    deliver(0);
    const writes = fx.writes;
    let notified = 0;
    for (const tab of tabs) tab.subscribe(() => notified++);
    deliver(0);
    deliver(1);
    assert.equal(fx.writes, writes, "no further writes");
    assert.equal(notified, 0, "no further notifications");
    assert.deepEqual(L.view(tabs[0].get()), { k1: "A", k2: "B" });
    assert.equal(canonical(tabs[0].get()), canonical(tabs[1].get()));
  });

  test(`${L.name}: a byte-identical clone does not write or notify`, () => {
    const d = device(L.lane);
    d.store.transact((t) => L.edit(t, "k1", "A"));
    let notified = 0;
    d.store.subscribe(() => notified++);
    d.store.transact((t) => ({ ...t }));
    assert.equal(notified, 0);
  });
}

test("publisher: isCurrent gates the first send and the reconnect retry", async () => {
  const sends = [];
  let current = true;
  const session = {
    generation: () => 1,
    ownership: () => 1,
    pendingEvents: new Map(),
    send: async (payload) => {
      sends.push(payload);
      throw new Error("socket closed");
    },
    reconnect: async () => {
      current = false; // lane changed while reconnecting
      return 1;
    },
    normalizeError: (e) => e,
    recoverSocketFailure: (e) => e,
  };
  Object.assign(globalThis.window, { setTimeout, clearTimeout });
  const event = { id: "e1" };
  await assert.rejects(
    publishSessionEvent(session, event, "t", "s", () => current),
    { message: PUBLISH_CANCELED },
  );
  assert.equal(sends.length, 1, "retry never sent");
  assert.equal(session.pendingEvents.size, 0);
  await assert.rejects(
    publishSessionEvent(session, event, "t", "s", () => false),
    { message: PUBLISH_CANCELED },
  );
  assert.equal(sends.length, 1, "first send never sent");
});
