import assert from "node:assert/strict";
import test from "node:test";

import {
  entriesEqual,
  isDefaultEntry,
  mergeStores,
  parseNotifyEntry,
  parseNotifyPrefsPayload,
  readChannelNotifyPrefsStore,
  setChannelEntry,
  storageKey,
  storesEqual,
  writeChannelNotifyPrefsStore,
} from "./channelNotifyPrefsStorage.ts";

// ── parseNotifyPrefsPayload ───────────────────────────────────────────────────

test("parse: valid payload keeps every known field", () => {
  const payload = {
    version: 1,
    channels: {
      "chan-1": { level: "mentions", updatedAt: 10 },
      "chan-2": {
        level: "mute",
        muteUntil: 500,
        desktop: false,
        followAllThreads: true,
        broadcasts: false,
        updatedAt: 20,
      },
    },
  };
  assert.deepEqual(parseNotifyPrefsPayload(payload), payload);
});

test("parse: rejects non-object and wrong-version payloads", () => {
  assert.equal(parseNotifyPrefsPayload(null), null);
  assert.equal(parseNotifyPrefsPayload("nope"), null);
  assert.equal(parseNotifyPrefsPayload(42), null);
  assert.equal(parseNotifyPrefsPayload({ channels: {} }), null);
  assert.equal(parseNotifyPrefsPayload({ version: 2, channels: {} }), null);
});

test("parse: missing or malformed channels map yields an empty store", () => {
  assert.deepEqual(parseNotifyPrefsPayload({ version: 1 }), {
    version: 1,
    channels: {},
  });
  assert.deepEqual(parseNotifyPrefsPayload({ version: 1, channels: [] }), {
    version: 1,
    channels: {},
  });
});

test("parse: drops entries without a usable updatedAt", () => {
  const result = parseNotifyPrefsPayload({
    version: 1,
    channels: {
      good: { level: "mute", updatedAt: 5 },
      noTimestamp: { level: "mute" },
      nanTimestamp: { level: "mute", updatedAt: Number.NaN },
      negative: { level: "mute", updatedAt: -1 },
      notAnObject: "mute",
      arrayEntry: [1, 2],
    },
  });
  assert.deepEqual(Object.keys(result.channels), ["good"]);
});

test("parse: drops malformed known fields but keeps the entry", () => {
  const result = parseNotifyPrefsPayload({
    version: 1,
    channels: {
      "chan-1": {
        level: "loud",
        muteUntil: "soon",
        desktop: "yes",
        followAllThreads: 1,
        broadcasts: null,
        updatedAt: 7,
      },
    },
  });
  assert.deepEqual(result.channels["chan-1"], { updatedAt: 7 });
});

test("parse: preserves unknown fields on entries it keeps (forward compat)", () => {
  const result = parseNotifyPrefsPayload({
    version: 1,
    channels: {
      "chan-1": {
        level: "mentions",
        mobile: false,
        futureThing: 3,
        updatedAt: 9,
      },
    },
  });
  assert.deepEqual(result.channels["chan-1"], {
    level: "mentions",
    mobile: false,
    futureThing: 3,
    updatedAt: 9,
  });
});

test("parseNotifyEntry: ignores a zero muteUntil", () => {
  assert.deepEqual(parseNotifyEntry({ muteUntil: 0, updatedAt: 4 }), {
    updatedAt: 4,
  });
});

// ── hydration republish predicate (what applyRemote gates the republish on) ───

test("merge/storesEqual: a local edit the remote blob lacks is detected, then settles", () => {
  // The F2 sequence at the store level: a remote blob already exists from an
  // earlier session, the user edits #random, and the debounce window is cut
  // short (community switch / reload) so nothing pending survives. On return,
  // the merge keeps the local edit — and comparing against the remote blob is
  // what tells the hook it still has to publish.
  const remote = {
    version: 1,
    channels: { eng: { level: "mentions", updatedAt: 100 } },
  };
  const localAfterLostDebounce = {
    version: 1,
    channels: {
      eng: { level: "mentions", updatedAt: 100 },
      random: { level: "mute", updatedAt: 200 },
    },
  };

  const merged = mergeStores(localAfterLostDebounce, remote);
  assert.deepEqual(merged.channels.random, { level: "mute", updatedAt: 200 });
  assert.equal(storesEqual(merged, remote), false);

  // Terminates: once our republished blob comes back on the subscription, the
  // merge result equals it and the hook stops republishing.
  assert.equal(storesEqual(mergeStores(merged, merged), merged), true);
});

test("merge/storesEqual: a remote blob that already holds every local entry needs no republish", () => {
  const store = {
    version: 1,
    channels: { eng: { level: "mute", desktop: false, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { eng: { level: "mute", desktop: false, updatedAt: 100 } },
  };
  assert.equal(storesEqual(mergeStores(store, remote), remote), true);
});

// ── updatedAt clock-skew clamp ────────────────────────────────────────────────

test("parse: an in-tolerance skewed updatedAt is preserved verbatim", () => {
  const now = 1_800_000_000;
  const parsed = parseNotifyPrefsPayload(
    {
      version: 1,
      channels: { eng: { level: "mute", updatedAt: now + 120 } },
    },
    now,
  );
  assert.equal(parsed.channels.eng.updatedAt, now + 120);
});

test("parse: a far-future updatedAt is clamped, keeping every other field", () => {
  const now = 1_800_000_000;
  const parsed = parseNotifyPrefsPayload(
    {
      version: 1,
      channels: {
        eng: {
          level: "mute",
          muteUntil: now + 315_360_000,
          desktop: false,
          broadcasts: false,
          followAllThreads: true,
          mobile: false,
          updatedAt: now + 315_360_000,
        },
      },
    },
    now,
  );
  assert.deepEqual(parsed.channels.eng, {
    level: "mute",
    // muteUntil is a legitimate absolute future timestamp — never clamped.
    muteUntil: now + 315_360_000,
    desktop: false,
    broadcasts: false,
    followAllThreads: true,
    mobile: false,
    updatedAt: now + 3_600,
  });
});

test("merge: a local edit beats a clamped far-future remote entry", () => {
  const now = 1_800_000_000;
  // One of the user's own devices has a clock set to 2030 and published a mute.
  const remote = parseNotifyPrefsPayload(
    {
      version: 1,
      channels: { eng: { level: "mute", updatedAt: now + 315_360_000 } },
    },
    now,
  );
  // The correctly-clocked device then picks "All new posts".
  const local = {
    version: 1,
    channels: { eng: { level: "all", updatedAt: now + 3_601 } },
  };
  assert.deepEqual(mergeStores(local, remote).channels.eng, {
    level: "all",
    updatedAt: now + 3_601,
  });
});

// ── merge (per-channel max-updatedAt LWW, local wins ties) ────────────────────

test("merge: unions keys and keeps the newer entry per channel", () => {
  const local = {
    version: 1,
    channels: {
      both: { level: "mentions", updatedAt: 30 },
      localOnly: { level: "mute", updatedAt: 5 },
    },
  };
  const remote = {
    version: 1,
    channels: {
      both: { level: "mute", updatedAt: 20 },
      remoteOnly: { desktop: false, updatedAt: 8 },
    },
  };
  assert.deepEqual(mergeStores(local, remote), {
    version: 1,
    channels: {
      both: { level: "mentions", updatedAt: 30 },
      localOnly: { level: "mute", updatedAt: 5 },
      remoteOnly: { desktop: false, updatedAt: 8 },
    },
  });
});

test("merge: local wins ties", () => {
  const merged = mergeStores(
    { version: 1, channels: { c: { level: "mentions", updatedAt: 10 } } },
    { version: 1, channels: { c: { level: "mute", updatedAt: 10 } } },
  );
  assert.deepEqual(merged.channels.c, { level: "mentions", updatedAt: 10 });
});

test("merge: newer remote entry replaces the whole local entry (no field merge)", () => {
  const merged = mergeStores(
    {
      version: 1,
      channels: { c: { level: "mute", desktop: false, updatedAt: 10 } },
    },
    { version: 1, channels: { c: { level: "mentions", updatedAt: 11 } } },
  );
  assert.deepEqual(merged.channels.c, { level: "mentions", updatedAt: 11 });
});

test("merge: preserves unknown fields carried by the winning entry", () => {
  const merged = mergeStores(
    { version: 1, channels: { c: { level: "mute", updatedAt: 1 } } },
    { version: 1, channels: { c: { mobile: true, updatedAt: 2 } } },
  );
  assert.deepEqual(merged.channels.c, { mobile: true, updatedAt: 2 });
});

// ── sparse entries ────────────────────────────────────────────────────────────

test("isDefaultEntry: default-valued entries, explicit or implicit", () => {
  assert.equal(isDefaultEntry({ updatedAt: 1 }), true);
  assert.equal(
    isDefaultEntry({
      level: "all",
      desktop: true,
      followAllThreads: false,
      broadcasts: true,
      updatedAt: 1,
    }),
    true,
  );
});

test("isDefaultEntry: any divergence, including unknown fields, is not default", () => {
  assert.equal(isDefaultEntry({ level: "mentions", updatedAt: 1 }), false);
  assert.equal(isDefaultEntry({ muteUntil: 99, updatedAt: 1 }), false);
  assert.equal(isDefaultEntry({ desktop: false, updatedAt: 1 }), false);
  assert.equal(isDefaultEntry({ followAllThreads: true, updatedAt: 1 }), false);
  assert.equal(isDefaultEntry({ broadcasts: false, updatedAt: 1 }), false);
  // Unknown fields must never be pruned away.
  assert.equal(isDefaultEntry({ mobile: false, updatedAt: 1 }), false);
});

test("setChannelEntry: does not materialize a default entry for an untouched channel", () => {
  const store = { version: 1, channels: {} };
  assert.equal(
    setChannelEntry(store, "c", { level: "all", updatedAt: 5 }),
    store,
  );
});

test("setChannelEntry: materializes a default entry over a non-default one (LWW tombstone)", () => {
  const store = {
    version: 1,
    channels: { c: { level: "mute", updatedAt: 5 } },
  };
  const next = setChannelEntry(store, "c", { level: "all", updatedAt: 9 });
  assert.deepEqual(next.channels.c, { level: "all", updatedAt: 9 });
});

test("setChannelEntry: prunes a default entry once the prior entry is also default", () => {
  const store = {
    version: 1,
    channels: {
      c: { level: "all", updatedAt: 9 },
      other: { level: "mute", updatedAt: 1 },
    },
  };
  const next = setChannelEntry(store, "c", { updatedAt: 12 });
  assert.deepEqual(Object.keys(next.channels), ["other"]);
});

test("setChannelEntry: stores diverging entries and leaves siblings alone", () => {
  const store = {
    version: 1,
    channels: { other: { level: "mute", updatedAt: 1 } },
  };
  const next = setChannelEntry(store, "c", { muteUntil: 500, updatedAt: 12 });
  assert.deepEqual(next.channels, {
    other: { level: "mute", updatedAt: 1 },
    c: { muteUntil: 500, updatedAt: 12 },
  });
});

// ── publish-dedup equality (all fields, not just the level) ───────────────────

test("entriesEqual: compares every field, including unknown ones", () => {
  assert.equal(
    entriesEqual(
      { level: "mute", updatedAt: 1 },
      { level: "mute", updatedAt: 1 },
    ),
    true,
  );
  assert.equal(
    entriesEqual(
      { level: "mute", updatedAt: 1 },
      { level: "mute", updatedAt: 2 },
    ),
    false,
  );
  assert.equal(
    entriesEqual(
      { level: "mute", desktop: false, updatedAt: 1 },
      { level: "mute", updatedAt: 1 },
    ),
    false,
  );
  assert.equal(
    entriesEqual(
      { level: "mute", mobile: true, updatedAt: 1 },
      { level: "mute", mobile: false, updatedAt: 1 },
    ),
    false,
  );
});

test("storesEqual: same keys and identical entries only", () => {
  const a = {
    version: 1,
    channels: { c: { level: "mute", updatedAt: 1 } },
  };
  assert.equal(storesEqual(a, structuredClone(a)), true);
  assert.equal(
    storesEqual(a, {
      version: 1,
      channels: { c: { level: "mute", updatedAt: 1 }, d: { updatedAt: 2 } },
    }),
    false,
  );
  assert.equal(
    storesEqual(a, {
      version: 1,
      channels: { d: { level: "mute", updatedAt: 1 } },
    }),
    false,
  );
  // A toggle that only changes desktop must not be deduped away.
  assert.equal(
    storesEqual(a, {
      version: 1,
      channels: { c: { level: "mute", desktop: false, updatedAt: 1 } },
    }),
    false,
  );
});

// ── relay-scoped localStorage key ─────────────────────────────────────────────

function withFakeLocalStorage(run) {
  const store = new Map();
  const previousWindow = globalThis.window;
  globalThis.window = {
    localStorage: {
      getItem: (key) => (store.has(key) ? store.get(key) : null),
      setItem: (key, value) => store.set(key, value),
      removeItem: (key) => store.delete(key),
    },
  };
  try {
    run(store);
  } finally {
    globalThis.window = previousWindow;
  }
}

test("storageKey: scoped to the normalized relay and the pubkey", () => {
  assert.equal(
    storageKey("pk", "wss://Relay.Example.com/"),
    "buzz-channel-notify-prefs.v1:wss%3A%2F%2Frelay.example.com:pk",
  );
  assert.notEqual(
    storageKey("pk", "wss://a.example"),
    storageKey("pk", "wss://b.example"),
  );
});

test("read/write: round-trips per relay without cross-relay bleed", () => {
  withFakeLocalStorage(() => {
    const store = {
      version: 1,
      channels: { c: { level: "mentions", updatedAt: 3 } },
    };
    assert.equal(
      writeChannelNotifyPrefsStore("pk", "wss://a.example", store),
      true,
    );
    assert.deepEqual(
      readChannelNotifyPrefsStore("pk", "wss://a.example"),
      store,
    );
    assert.deepEqual(readChannelNotifyPrefsStore("pk", "wss://b.example"), {
      version: 1,
      channels: {},
    });
  });
});

test("read: corrupt or wrong-version localStorage falls back to the default store", () => {
  withFakeLocalStorage((raw) => {
    raw.set(storageKey("pk", "wss://a.example"), "{not json");
    assert.deepEqual(readChannelNotifyPrefsStore("pk", "wss://a.example"), {
      version: 1,
      channels: {},
    });
    raw.set(
      storageKey("pk", "wss://a.example"),
      JSON.stringify({ version: 2, channels: { c: { updatedAt: 1 } } }),
    );
    assert.deepEqual(readChannelNotifyPrefsStore("pk", "wss://a.example"), {
      version: 1,
      channels: {},
    });
  });
});
