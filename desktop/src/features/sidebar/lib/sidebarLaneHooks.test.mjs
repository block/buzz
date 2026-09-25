import assert from "node:assert/strict";
import { after, before, beforeEach, mock, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html>", { url: "http://localhost" });
before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
});
after(() => dom.window.close());

const PK = "f".repeat(64);
const RELAY = "wss://r.test";
let fx;
let seq = 0;
const ev = (dTag, json, createdAt) => ({
  id: String(++seq).padStart(64, "0"),
  pubkey: PK,
  kind: 30078,
  created_at: createdAt,
  content: JSON.stringify(json),
  tags: [["d", dTag]],
  sig: "s",
});

async function harness() {
  const rtl = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  mock.method(relayClient, "fetchEvents", async (filter) => {
    if (fx.onFetch) return fx.onFetch();
    const head = fx.heads[filter["#d"][0]];
    if (fx.gate) await fx.gate;
    return head ? [head] : [];
  });
  mock.method(relayClient, "subscribeLive", async () => async () => {});
  mock.method(relayClient, "subscribeToReconnects", (fn) => {
    fx.reconnect = fn;
    return () => {};
  });
  mock.method(relayClient, "publishEvent", async (event, _t, _e, isCurrent) => {
    if (fx.holdPublish) await fx.holdPublish;
    if (isCurrent && !isCurrent()) throw new Error("canceled");
    fx.published.push(event);
    fx.heads[event.tags[0][1]] = event;
  });
  mock.method(console, "warn", () => {});
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd === "nip44_encrypt_to_self") return args.plaintext;
      if (cmd === "nip44_decrypt_from_self") return args.ciphertext;
      if (cmd === "sign_event")
        return JSON.stringify({
          ...ev(args.tags[0][1], JSON.parse(args.content), args.createdAt),
          content: args.content,
        });
      throw new Error(`unmocked ${cmd}`);
    },
  };
  // Advance fake time in steps, letting async work between timers settle.
  const advance = async (ms) => {
    for (let t = 0; t < ms; t += 1_000) {
      await rtl.act(async () => {
        mock.timers.tick(1_000);
        for (let i = 0; i < 10; i++) await new Promise(setImmediate);
      });
    }
  };
  return { ...rtl, advance };
}

beforeEach(() => {
  window.localStorage.clear();
  mock.restoreAll();
  mock.timers.reset();
  mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1e12 });
  fx = { heads: {}, published: [], reconnect: null };
});

for (const [name, modPath, hookName, dTag, idsKey] of [
  [
    "stars",
    "./useChannelStars.ts",
    "useChannelStars",
    "channel-stars",
    "starredChannelIds",
  ],
  [
    "mutes",
    "./useChannelMutes.ts",
    "useChannelMutes",
    "channel-mutes",
    "mutedChannelIds",
  ],
]) {
  test(`${name}: the real hook recovers a missed head on the 5 s and steady 60 s ticks`, async () => {
    const { renderHook, advance, cleanup } = await harness();
    const hook = (await import(modPath))[hookName];
    const entry = (ids) =>
      Object.fromEntries(
        ids.map((id) => [
          id,
          { [name === "stars" ? "starred" : "muted"]: true, updatedAt: 1 },
        ]),
      );
    const { result } = renderHook(() => hook(PK, RELAY));
    await advance(1_000);
    assert.equal(result.current[idsKey].size, 0);
    fx.heads[dTag] = ev(dTag, { version: 1, channels: entry(["c1"]) }, 100);
    await advance(5_000);
    assert.deepEqual([...result.current[idsKey]], ["c1"]);
    await advance(10_000 + 30_000); // back-off climbs to its steady 60 s
    fx.heads[dTag] = ev(
      dTag,
      { version: 1, channels: entry(["c1", "c2"]) },
      200,
    );
    await advance(60_000);
    assert.deepEqual([...result.current[idsKey]].sort(), ["c1", "c2"]);
    cleanup();
  });
}

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => ([resolve, reject] = [res, rej]));
  return { promise, resolve, reject };
};

for (const [name, modPath, hookName, dTag, idsKey, verb, field] of [
  [
    "stars",
    "./useChannelStars.ts",
    "useChannelStars",
    "channel-stars",
    "starredChannelIds",
    "starChannel",
    "starred",
  ],
  [
    "mutes",
    "./useChannelMutes.ts",
    "useChannelMutes",
    "channel-mutes",
    "mutedChannelIds",
    "muteChannel",
    "muted",
  ],
]) {
  const payload = (ids, updatedAt = 1) => ({
    version: 1,
    channels: Object.fromEntries(
      ids.map((id) => [id, { [field]: true, updatedAt }]),
    ),
  });
  const headIds = () =>
    Object.keys(JSON.parse(fx.heads[dTag].content).channels).sort();
  const visible = () =>
    document.dispatchEvent(new window.Event("visibilitychange"));
  const setup = async () => {
    Object.defineProperty(document, "visibilityState", {
      value: "visible",
      configurable: true,
    });
    const h = await harness();
    const hook = (await import(modPath))[hookName];
    return { ...h, hook };
  };

  test(`${name}: an older ACK leaves a newer edit pending through recovery`, async () => {
    const { renderHook, act, advance, cleanup, hook } = await setup();
    const { result } = renderHook(() => hook(PK, RELAY));
    await advance(1_000);
    act(() => result.current[verb]("c1")); // edit A
    const ack = deferred();
    fx.holdPublish = ack.promise;
    await advance(2_000); // A is at the socket, its ACK held
    act(() => result.current[verb]("c2")); // edit B, deadline +2 s
    fx.holdPublish = null;
    await act(async () => ack.resolve());
    await act(async () => visible()); // recovery before B's deadline
    await advance(3_000);
    assert.deepEqual(headIds(), ["c1", "c2"], "B reached the relay");
    cleanup();
  });

  test(`${name}: an identical-payload exit leaves a newer edit pending through recovery`, async () => {
    const { renderHook, act, advance, cleanup, hook } = await setup();
    const { result } = renderHook(() => hook(PK, RELAY));
    await advance(1_000);
    act(() => result.current[verb]("c1")); // A
    const ack = deferred();
    fx.holdPublish = ack.promise;
    await advance(2_000);
    await act(async () => fx.reconnect()); // re-queues pending A
    fx.holdPublish = null;
    await act(async () => ack.resolve());
    const gate = deferred();
    fx.gate = gate.promise;
    await advance(2_000); // A's repeat waits in preflight
    act(() => result.current[verb]("c2")); // B
    fx.gate = null;
    await act(async () => gate.resolve()); // repeat of A exits as identical
    await act(async () => visible());
    await advance(3_000);
    assert.deepEqual(headIds(), ["c1", "c2"], "B reached the relay");
    cleanup();
  });

  for (const preflight of ["absent", "failed", "found", "edit"]) {
    const what =
      preflight === "edit"
        ? "a genuine edit survives the seed's retirement"
        : `an acquired seed yields to R (preflight ${preflight})`;
    test(`${name}: startup recovery sees R; ${what}`, async () => {
      const { renderHook, act, advance, cleanup, hook } = await setup();
      const { storageKey } = await import(
        `./channel${name === "stars" ? "Stars" : "Mutes"}Storage.ts`
      );
      window.localStorage.setItem(
        storageKey(PK),
        JSON.stringify(payload(["s1"])),
      );
      const R = ev(dTag, payload(["r1"], 2), 1e9 + 30); // future-dated
      fx.heads[dTag] = R;
      const calls = [];
      fx.onFetch = () => {
        const d = deferred();
        calls.push(d);
        return d.promise;
      };
      const { result } = renderHook(() => hook(PK, RELAY));
      await act(async () => calls[0].resolve([])); // bootstrap: absent, seeds S
      if (preflight === "edit") {
        act(() => result.current[verb]("c9"));
        await act(async () => calls[1].resolve([R]));
        await advance(2_000);
        await act(async () => calls[2].resolve([R]));
      } else {
        await advance(2_000); // seed acquired; its preflight waits
        await act(async () => calls[1].resolve([R])); // recovery observes R
        if (preflight === "failed") calls[2].reject(new Error("offline"));
        else calls[2].resolve(preflight === "found" ? [R] : []);
      }
      await advance(3_000);
      if (preflight === "edit") {
        assert.deepEqual(headIds(), ["c9", "r1", "s1"]);
      } else {
        assert.equal(fx.published.length, 0, "the seed never published");
        assert.equal(fx.heads[dTag], R, "R stays the relay head");
        assert.deepEqual([...result.current[idsKey]].sort(), ["r1", "s1"]);
      }
      cleanup();
    });
  }
}

test("sections hook: 60 s tick, reconnect, cross-tab, pending edit and in-flight unmount", async () => {
  const { renderHook, act, advance, cleanup } = await harness();
  const { useChannelSections } = await import("./useChannelSections.ts");
  const { storageKey } = await import("./channelSectionsStorage.ts");
  const legacy = (names) => ({
    version: 1,
    sections: names.map((n, order) => ({ id: n, name: n, order })),
    assignments: {},
  });
  fx.heads["channel-sections"] = ev("channel-sections", legacy(["a"]), 100);
  const { result, unmount } = renderHook(() => useChannelSections(PK, RELAY));
  await advance(1_000);
  const names = () => result.current.sections.map((s) => s.name);
  assert.deepEqual(names(), ["a"]);

  // Missed live event after the back-off reached 60 s (ticks at 5/15/45 s):
  // only the steady tick at 105 s picks it up.
  await advance(45_000);
  fx.heads["channel-sections"] = ev(
    "channel-sections",
    legacy(["a", "b"]),
    200,
  );
  await advance(58_000);
  assert.deepEqual(names(), ["a"], "not yet: no tick before 105 s");
  await advance(2_000);
  assert.deepEqual(names(), ["a", "b"]);

  // Reconnect re-reads.
  fx.heads["channel-sections"] = ev(
    "channel-sections",
    legacy(["a", "b", "c"]),
    300,
  );
  await act(async () => fx.reconnect());
  await advance(1_000);
  assert.deepEqual(names(), ["a", "b", "c"]);

  // Another tab's write merges (never replaces).
  const other = legacy(["d"]);
  await act(async () =>
    window.dispatchEvent(
      new window.StorageEvent("storage", {
        key: storageKey(PK, RELAY),
        newValue: JSON.stringify(other),
      }),
    ),
  );
  assert.deepEqual(names().sort(), ["a", "b", "c", "d"]);

  // A read (here, reconnect) during the edit debounce merges but cannot
  // publish early or shorten the debounce.
  const before = fx.published.length;
  act(() => void result.current.createSection("e"));
  fx.heads["channel-sections"] = ev(
    "channel-sections",
    legacy(["a", "b", "c", "f"]),
    400,
  );
  await act(async () => fx.reconnect());
  await advance(1_000);
  assert.equal(fx.published.length, before, "held by the debounce");
  assert.ok(names().includes("f") && names().includes("e"));

  // Unmount while the publish is in flight: nothing reaches the socket.
  let release;
  fx.holdPublish = new Promise((r) => (release = r));
  await advance(2_000);
  unmount();
  release();
  await advance(1_000);
  assert.equal(fx.published.length, before);
  cleanup();
});

test("sections hook: create appends past gaps; reorder completes against the live list", async () => {
  const { renderHook, act, advance, cleanup } = await harness();
  const { useChannelSections } = await import("./useChannelSections.ts");
  const { storageKey } = await import("./channelSectionsStorage.ts");
  const { result } = renderHook(() => useChannelSections(PK, RELAY));
  await advance(1_000);
  const names = () => result.current.sections.map((s) => s.name);
  const ids = {};
  for (const n of ["a", "b", "c"])
    act(() => {
      ids[n] = result.current.createSection(n).id;
    });
  act(() => result.current.deleteSection(ids.b)); // canonical ranks 0 and 2
  act(() => void result.current.createSection("d"));
  assert.deepEqual(names(), ["a", "c", "d"], "new section sorts last");

  // A section merged in from another tab after the drag snapshot was taken.
  const snapshot = result.current.sections.map((s) => s.id);
  await act(async () =>
    window.dispatchEvent(
      new window.StorageEvent("storage", {
        key: storageKey(PK, RELAY),
        newValue: JSON.stringify({
          version: 1,
          sections: [{ id: "x", name: "x", order: 9 }],
          assignments: {},
        }),
      }),
    ),
  );
  act(() =>
    result.current.reorderSections([...snapshot].reverse().concat("gone")),
  );
  assert.deepEqual(names(), ["d", "c", "a", "x"]);
  const orders = JSON.parse(window.localStorage.getItem(storageKey(PK, RELAY)))
    .meta.s;
  assert.ok(orders.x.order, "the missing live section got an order register");
  cleanup();
});

const UPGRADE_LANES = [
  {
    name: "sections",
    dTag: "channel-sections",
    load: async () => [
      (await import("./useChannelSections.ts")).useChannelSections,
      (await import("./channelSectionsStorage.ts")).storageKey,
    ],
    legacy: (name) => ({
      version: 1,
      sections: [{ id: "x", name, order: 0 }],
      assignments: {},
    }),
    ui: (r) => r.sections[0]?.name,
    fromDoc: (doc) => doc.sections[0]?.name,
    edit: (r, v) => r.renameSection("x", v),
    values: ["Eng", "Platform", "Mine"],
  },
  {
    name: "sort",
    dTag: "channel-sort",
    load: async () => [
      (await import("./useChannelSortPreference.ts")).useChannelSortPreference,
      (await import("./channelSortPreference.ts")).storageKey,
    ],
    legacy: (mode) => ({ version: 1, groups: { channels: mode } }),
    ui: (r) => r.sortModeFor("channels"),
    fromDoc: (doc) => doc.groups.channels,
    edit: (r, v) => {
      r.setSortModeFor("channels", "alpha"); // authored, then back
      r.setSortModeFor("channels", v);
    },
    values: ["recent", "alpha", "recent"],
  },
];

for (const L of UPGRADE_LANES) {
  for (const edited of [false, true]) {
    const what = edited
      ? "a genuine local edit still beats the relay"
      : "a cache-only import yields to the relay head";
    test(`${L.name} first upgrade: ${what}`, async () => {
      const { renderHook, act, advance, cleanup } = await harness();
      const [hook, key] = await L.load();
      const [cached, remote, mine] = L.values;
      window.localStorage.setItem(
        key(PK, RELAY),
        JSON.stringify(L.legacy(cached)),
      );
      fx.heads[L.dTag] = ev(L.dTag, L.legacy(remote), 100);
      let open;
      fx.gate = new Promise((r) => (open = r));
      const { result } = renderHook(() => hook(PK, RELAY));
      if (edited) act(() => L.edit(result.current, mine));
      fx.gate = null;
      open();
      await advance(5_000);
      const want = edited ? mine : remote;
      assert.equal(L.ui(result.current), want, "UI");
      const cache = JSON.parse(window.localStorage.getItem(key(PK, RELAY)));
      assert.equal(L.fromDoc(cache), want, "cache");
      const head = JSON.parse(fx.heads[L.dTag].content);
      assert.equal(L.fromDoc(head), want, "relay head");
      assert.ok(head.meta, "a fresh reader gets the registers");
      cleanup();
    });
  }
}
