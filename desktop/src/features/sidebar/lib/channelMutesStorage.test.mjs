import assert from "node:assert/strict";
import test from "node:test";

import {
  boundMuteStore,
  MAX_CHANNEL_MUTE_ENTRIES,
  parseMutePayload,
  mergeStores,
  mutedChannelIdsFromStore,
} from "./channelMutesStorage.ts";

// ── parseMutePayload ──────────────────────────────────────────────────────────

test("parseMutePayload: valid payload with channels returns store", () => {
  const payload = {
    version: 1,
    channels: {
      "chan-1": { muted: true, updatedAt: 1000 },
      "chan-2": { muted: false, updatedAt: 2000 },
    },
  };
  const result = parseMutePayload(payload);
  assert.deepEqual(result, {
    version: 1,
    channels: {
      "chan-1": { muted: true, updatedAt: 1000 },
      "chan-2": { muted: false, updatedAt: 2000 },
    },
  });
});

test("parseMutePayload: missing version returns null", () => {
  assert.equal(
    parseMutePayload({ channels: { "chan-1": { muted: true, updatedAt: 1 } } }),
    null,
  );
});

test("parseMutePayload: wrong version returns null", () => {
  assert.equal(
    parseMutePayload({
      version: 2,
      channels: { "chan-1": { muted: true, updatedAt: 1 } },
    }),
    null,
  );
});

test("parseMutePayload: null input returns null", () => {
  assert.equal(parseMutePayload(null), null);
});

test("parseMutePayload: non-object input returns null", () => {
  assert.equal(parseMutePayload("string"), null);
  assert.equal(parseMutePayload(42), null);
  assert.equal(parseMutePayload(true), null);
});

test("parseMutePayload: malformed channel entries missing muted/updatedAt are filtered out", () => {
  const payload = {
    version: 1,
    channels: {
      "no-muted": { updatedAt: 1000 },
      "no-updated-at": { muted: true },
      valid: { muted: false, updatedAt: 500 },
      "muted-wrong-type": { muted: "yes", updatedAt: 1000 },
      "updated-at-wrong-type": { muted: true, updatedAt: "now" },
      null: null,
    },
  };
  const result = parseMutePayload(payload);
  assert.deepEqual(result, {
    version: 1,
    channels: {
      valid: { muted: false, updatedAt: 500 },
    },
  });
});

test("parseMutePayload: NaN/Infinity/negative updatedAt entries are filtered out", () => {
  const payload = {
    version: 1,
    channels: {
      nan: { muted: true, updatedAt: NaN },
      inf: { muted: true, updatedAt: Infinity },
      "neg-inf": { muted: true, updatedAt: -Infinity },
      neg: { muted: true, updatedAt: -1 },
      valid: { muted: true, updatedAt: 100 },
    },
  };
  const result = parseMutePayload(payload);
  assert.deepEqual(result, {
    version: 1,
    channels: { valid: { muted: true, updatedAt: 100 } },
  });
});

test("parseMutePayload: empty channels returns store with empty channels", () => {
  const result = parseMutePayload({ version: 1, channels: {} });
  assert.deepEqual(result, { version: 1, channels: {} });
});

test("parseMutePayload: version 1 with no channels key returns store with empty channels", () => {
  const result = parseMutePayload({ version: 1 });
  assert.deepEqual(result, { version: 1, channels: {} });
});

// ── mergeStores ───────────────────────────────────────────────────────────────

test("mergeStores: non-overlapping channels returns union of both", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { muted: true, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-b": { muted: false, updatedAt: 200 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result, {
    version: 1,
    channels: {
      "chan-a": { muted: true, updatedAt: 100 },
      "chan-b": { muted: false, updatedAt: 200 },
    },
  });
});

test("mergeStores: overlapping channel with remote newer takes remote", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { muted: false, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { muted: true, updatedAt: 200 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], { muted: true, updatedAt: 200 });
});

test("mergeStores: overlapping channel with local newer takes local", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { muted: true, updatedAt: 300 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { muted: false, updatedAt: 100 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], { muted: true, updatedAt: 300 });
});

test("mergeStores: overlapping channel with same updatedAt local wins", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { muted: true, updatedAt: 500 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { muted: false, updatedAt: 500 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], { muted: true, updatedAt: 500 });
});

test("mergeStores: unmute with higher updatedAt overrides mute", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { muted: true, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { muted: false, updatedAt: 999 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], { muted: false, updatedAt: 999 });
});

test("mergeStores: empty local returns remote entries", () => {
  const local = { version: 1, channels: {} };
  const remote = {
    version: 1,
    channels: { "chan-b": { muted: true, updatedAt: 42 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels, {
    "chan-b": { muted: true, updatedAt: 42 },
  });
});

test("mergeStores: empty remote returns local entries", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { muted: false, updatedAt: 10 } },
  };
  const remote = { version: 1, channels: {} };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels, {
    "chan-a": { muted: false, updatedAt: 10 },
  });
});

test("mergeStores: both empty returns empty", () => {
  const result = mergeStores(
    { version: 1, channels: {} },
    { version: 1, channels: {} },
  );
  assert.deepEqual(result, { version: 1, channels: {} });
});

test("boundMuteStore: retains newest entries regardless of muted value", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES }, (_, index) => [
      `active-${index}`,
      { muted: true, updatedAt: index + 1 },
    ]),
  );
  channels["old-false"] = { muted: false, updatedAt: 0 };
  channels["new-false"] = { muted: false, updatedAt: 9999 };

  const result = boundMuteStore({ version: 1, channels });

  assert.equal(Object.keys(result.channels).length, MAX_CHANNEL_MUTE_ENTRIES);
  assert.equal(result.channels["old-false"], undefined);
  assert.deepEqual(result.channels["new-false"], {
    muted: false,
    updatedAt: 9999,
  });
  assert.equal(result.channels["active-0"], undefined);
  assert.deepEqual(result.channels["active-1"], { muted: true, updatedAt: 2 });
});

test("boundMuteStore: uses channel ID as an updatedAt tie-breaker", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES + 1 }, (_, index) => [
      `channel-${String(MAX_CHANNEL_MUTE_ENTRIES - index).padStart(3, "0")}`,
      { muted: true, updatedAt: 1 },
    ]),
  );

  const result = boundMuteStore({ version: 1, channels });

  assert.equal(result.channels["channel-000"], undefined);
  assert.deepEqual(result.channels["channel-500"], {
    muted: true,
    updatedAt: 1,
  });
});

test("boundMuteStore: preserves a same-second mute mutation by key", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES }, (_, index) => [
      `z-channel-${String(index).padStart(3, "0")}`,
      { muted: true, updatedAt: 1 },
    ]),
  );
  channels["a-target"] = { muted: true, updatedAt: 1 };

  const result = boundMuteStore({ version: 1, channels }, "a-target");

  assert.equal(Object.keys(result.channels).length, MAX_CHANNEL_MUTE_ENTRIES);
  assert.deepEqual(result.channels["a-target"], {
    muted: true,
    updatedAt: 1,
  });
  assert.equal(result.channels["z-channel-000"], undefined);
});

test("boundMuteStore: preserves a same-second unmute mutation by key", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES }, (_, index) => [
      `z-channel-${String(index).padStart(3, "0")}`,
      { muted: true, updatedAt: 1 },
    ]),
  );
  channels["a-target"] = { muted: false, updatedAt: 1 };

  const result = boundMuteStore({ version: 1, channels }, "a-target");

  assert.equal(Object.keys(result.channels).length, MAX_CHANNEL_MUTE_ENTRIES);
  assert.deepEqual(result.channels["a-target"], {
    muted: false,
    updatedAt: 1,
  });
  assert.equal(result.channels["z-channel-000"], undefined);
});

test("mergeStores: a fresh at-capacity unmute defeats an older remote mute", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES }, (_, index) => [
      `active-${index}`,
      { muted: true, updatedAt: index + 1 },
    ]),
  );
  channels["unmuted"] = { muted: false, updatedAt: 9999 };
  const bounded = boundMuteStore({ version: 1, channels });

  const result = mergeStores(bounded, {
    version: 1,
    channels: { unmuted: { muted: true, updatedAt: 9998 } },
  });

  assert.deepEqual(result.channels.unmuted, {
    muted: false,
    updatedAt: 9999,
  });
});

test("mergeStores: evicted remote ID re-enters and the oldest state is re-trimmed", () => {
  const localChannels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES }, (_, index) => [
      `active-${index}`,
      { muted: true, updatedAt: index + 10 },
    ]),
  );
  const result = mergeStores(
    { version: 1, channels: localChannels },
    {
      version: 1,
      channels: {
        "evicted-id": { muted: true, updatedAt: 9999 },
        "active-0": { muted: false, updatedAt: 9998 },
      },
    },
  );

  assert.equal(Object.keys(result.channels).length, MAX_CHANNEL_MUTE_ENTRIES);
  assert.deepEqual(result.channels["evicted-id"], {
    muted: true,
    updatedAt: 9999,
  });
  assert.deepEqual(result.channels["active-0"], {
    muted: false,
    updatedAt: 9998,
  });
  assert.equal(result.channels["active-1"], undefined);
  assert.deepEqual(result.channels["active-2"], { muted: true, updatedAt: 12 });
});

// ── mutedChannelIdsFromStore ──────────────────────────────────────────────────

test("mutedChannelIdsFromStore: returns set of IDs where muted=true", () => {
  const store = {
    version: 1,
    channels: {
      "chan-a": { muted: true, updatedAt: 100 },
      "chan-b": { muted: true, updatedAt: 200 },
      "chan-c": { muted: false, updatedAt: 300 },
    },
  };
  const result = mutedChannelIdsFromStore(store);
  assert.equal(result.has("chan-a"), true);
  assert.equal(result.has("chan-b"), true);
  assert.equal(result.has("chan-c"), false);
  assert.equal(result.size, 2);
});

test("mutedChannelIdsFromStore: excludes IDs where muted=false", () => {
  const store = {
    version: 1,
    channels: {
      "chan-x": { muted: false, updatedAt: 1 },
      "chan-y": { muted: false, updatedAt: 2 },
    },
  };
  const result = mutedChannelIdsFromStore(store);
  assert.equal(result.size, 0);
});

test("mutedChannelIdsFromStore: empty channels returns empty set", () => {
  const result = mutedChannelIdsFromStore({ version: 1, channels: {} });
  assert.equal(result.size, 0);
});
