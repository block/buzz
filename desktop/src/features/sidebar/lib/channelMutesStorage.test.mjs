// Compact mutes lane adapter contract.
// Full merge-lane storage invariants are in mergeLaneStorage.shared.test.mjs.
// This file proves mutes-specific wiring PLUS duplicated mutes algebra not in the stars-based shared suite.

import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_CHANNEL_MUTE_ENTRIES,
  boundMuteStore,
  isMutesStoreSubsumedBy,
  mergeStores,
  mutedChannelIdsFromStore,
  parseMutePayload,
  storageKey,
} from "./channelMutesStorage.ts";

test("mutes adapter: value field is 'muted', key prefix is buzz-channel-mutes.v1", () => {
  const raw = {
    version: 1,
    channels: { a: { muted: true, updatedAt: 100, rev: 1 } },
  };
  const parsed = parseMutePayload(raw);
  assert.ok(parsed !== null, "valid mutes payload parses");
  assert.equal(parsed.channels.a.muted, true, "muted field preserved");
  assert.equal(
    storageKey("pk1"),
    "buzz-channel-mutes.v1:pk1",
    "storage key prefix",
  );
});

test("mutes adapter: idsFromStore projects muted=true entries", () => {
  const store = {
    version: 1,
    channels: {
      a: { muted: true, updatedAt: 1, rev: 0 },
      b: { muted: false, updatedAt: 2, rev: 0 },
    },
  };
  const ids = mutedChannelIdsFromStore(store);
  assert.ok(ids.has("a"), "muted channel in set");
  assert.ok(!ids.has("b"), "unmuted channel excluded");
});

test("mutes adapter: wrong value field (starred) is rejected by parser", () => {
  const starPayload = {
    version: 1,
    channels: { a: { starred: true, updatedAt: 100, rev: 1 } },
  };
  const result = parseMutePayload(starPayload);
  assert.deepEqual(
    result?.channels ?? {},
    {},
    "starred entry filtered as malformed",
  );
});
// These laws hold in channelStarsStorage too (exercised by mergeLaneStorage.shared),
// but the implementations are independent — a mute-specific copy-paste error
// cannot be caught by the stars suite.

function muteStore(channels) {
  return { version: 1, channels };
}
function muteEntry(muted, updatedAt, rev = 0) {
  return { muted, updatedAt, rev };
}

test("mutes mergeStores: winner selection algebra (all cases also verify commutativity)", () => {
  const check = (a, b, expected) => {
    assert.deepEqual(mergeStores(a, b).channels, expected, "a∪b");
    assert.deepEqual(mergeStores(b, a).channels, expected, "b∪a");
  };
  check(
    muteStore({ ch: muteEntry(true, 100, 0) }),
    muteStore({ ch: muteEntry(false, 200, 0) }),
    { ch: muteEntry(false, 200, 0) },
  );
  check(
    muteStore({ ch: muteEntry(true, 100, 1) }),
    muteStore({ ch: muteEntry(false, 100, 2) }),
    { ch: muteEntry(false, 100, 2) },
  );
  check(
    muteStore({ ch: muteEntry(false, 100, 1) }),
    muteStore({ ch: muteEntry(true, 100, 1) }),
    { ch: muteEntry(true, 100, 1) },
  );
  check(
    muteStore({ ch: muteEntry(true, 100, 0) }),
    muteStore({ ch: muteEntry(false, 100, 1) }),
    { ch: muteEntry(false, 100, 1) },
  );
  check(
    muteStore({ ch: muteEntry(true, 100, 3) }),
    muteStore({ ch: muteEntry(false, 200, 1) }),
    { ch: muteEntry(false, 200, 1) },
  );
});

test("mutes boundMuteStore: caps at MAX_CHANNEL_MUTE_ENTRIES, retains newest", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES + 2 }, (_, i) => [
      `ch${String(i).padStart(4, "0")}`,
      muteEntry(true, i, 0),
    ]),
  );
  const bounded = boundMuteStore({ version: 1, channels });
  assert.equal(
    Object.keys(bounded.channels).length,
    MAX_CHANNEL_MUTE_ENTRIES,
    "capped at limit",
  );
  // Oldest two (updatedAt 0 and 1) should be evicted.
  assert.ok(!bounded.channels["ch0000"], "oldest evicted");
  assert.ok(!bounded.channels["ch0001"], "second-oldest evicted");
});

test("mutes boundMuteStore: preserved key survives even when oldest", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES + 1 }, (_, i) => [
      `ch${String(i).padStart(4, "0")}`,
      muteEntry(true, i, 0),
    ]),
  );
  const preservedKey = "ch0000"; // updatedAt=0, would normally be evicted
  const bounded = boundMuteStore({ version: 1, channels }, preservedKey);
  assert.ok(bounded.channels[preservedKey], "preserved key always retained");
  assert.equal(
    Object.keys(bounded.channels).length,
    MAX_CHANNEL_MUTE_ENTRIES,
    "still capped at limit",
  );
});

test("mutes isMutesStoreSubsumedBy: newer/identical head subsumes; older head does not", () => {
  const candidate = muteStore({ ch: muteEntry(true, 100, 1) });
  assert.ok(
    isMutesStoreSubsumedBy(
      candidate,
      muteStore({ ch: muteEntry(false, 200, 2) }),
    ),
    "head subsumes older candidate",
  );
  assert.ok(
    !isMutesStoreSubsumedBy(
      muteStore({ ch: muteEntry(true, 300, 5) }),
      muteStore({ ch: muteEntry(false, 200, 2) }),
    ),
    "newer candidate not subsumed",
  );
  assert.ok(
    isMutesStoreSubsumedBy(candidate, candidate),
    "identical is subsumed",
  );
});
