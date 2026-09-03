import assert from "node:assert/strict";
import test from "node:test";

import {
  createNavigationHistoryState,
  describeHistoryLocation,
  getBackHistoryEntries,
  getForwardHistoryEntries,
  recordHistoryVisit,
} from "./navigationHistory.ts";

function entry(index) {
  return { index, key: `key-${index}`, label: `Entry ${index}` };
}

function visit(state, index, key, label) {
  return recordHistoryVisit(state, { index, key, label });
}

test("back history returns the nearest ten entries in reverse order", () => {
  const entriesByIndex = new Map(
    Array.from({ length: 13 }, (_, index) => [index, entry(index)]),
  );

  assert.deepEqual(
    getBackHistoryEntries(entriesByIndex, 13).map(({ index }) => index),
    [12, 11, 10, 9, 8, 7, 6, 5, 4, 3],
  );
});

test("forward history returns the nearest ten entries in navigation order", () => {
  const entriesByIndex = new Map(
    Array.from({ length: 14 }, (_, index) => [index, entry(index)]),
  );

  assert.deepEqual(
    getForwardHistoryEntries(entriesByIndex, 0, 13).map(({ index }) => index),
    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
  );
});

test("history labels identify channel and thread destinations", () => {
  const channels = [
    {
      id: "channel-a",
      name: "general",
      channelType: "stream",
    },
    {
      id: "dm-a",
      name: "Ada Lovelace",
      channelType: "dm",
    },
  ];

  assert.equal(
    describeHistoryLocation(
      { pathname: "/channels/channel-a", search: {} },
      channels,
    ),
    "#general",
  );
  assert.equal(
    describeHistoryLocation(
      {
        pathname: "/channels/channel-a",
        search: { thread: "message-a" },
      },
      channels,
    ),
    "#general thread",
  );
  assert.equal(
    describeHistoryLocation(
      { pathname: "/channels/dm-a", search: {} },
      channels,
    ),
    "Ada Lovelace",
  );
});

test("replacing an entry mid-history keeps the forward entries", () => {
  // Inbox → #general → #random, then back to #general.
  let state = createNavigationHistoryState({
    index: 0,
    key: "key-inbox",
    label: "Inbox",
  });
  state = visit(state, 1, "key-general", "#general");
  state = visit(state, 2, "key-random", "#random");
  state = visit(state, 1, "key-general", "#general");

  // A `replace: true` navigation mints a fresh key at the same index while the
  // browser keeps entry 2 reachable with forward.
  state = visit(state, 1, "key-general-replaced", "#general thread");

  assert.equal(state.maxIndex, 2);
  assert.deepEqual(
    getForwardHistoryEntries(state.entriesByIndex, 1, state.maxIndex).map(
      ({ label }) => label,
    ),
    ["#random"],
  );
  assert.equal(state.entriesByIndex.get(1).label, "#general thread");
});

test("pushing from mid-history drops the stale forward entries", () => {
  let state = createNavigationHistoryState({
    index: 0,
    key: "key-inbox",
    label: "Inbox",
  });
  state = visit(state, 1, "key-general", "#general");
  state = visit(state, 2, "key-random", "#random");
  state = visit(state, 1, "key-general", "#general");
  state = visit(state, 0, "key-inbox", "Inbox");

  // Navigating anew from the inbox invalidates both entries ahead of it.
  state = visit(state, 1, "key-design", "#design");

  assert.equal(state.maxIndex, 1);
  assert.deepEqual([...state.entriesByIndex.keys()], [0, 1]);
  assert.deepEqual(
    getForwardHistoryEntries(state.entriesByIndex, 1, state.maxIndex),
    [],
  );
});

test("traversing back and forward leaves the tracked entries untouched", () => {
  let state = createNavigationHistoryState({
    index: 0,
    key: "key-inbox",
    label: "Inbox",
  });
  state = visit(state, 1, "key-general", "#general");
  state = visit(state, 2, "key-random", "#random");
  state = visit(state, 0, "key-inbox", "Inbox");

  assert.equal(state.maxIndex, 2);
  assert.deepEqual(
    getForwardHistoryEntries(state.entriesByIndex, 0, state.maxIndex).map(
      ({ label }) => label,
    ),
    ["#general", "#random"],
  );

  state = visit(state, 2, "key-random", "#random");

  assert.equal(state.maxIndex, 2);
  assert.deepEqual(
    getBackHistoryEntries(state.entriesByIndex, 2).map(({ label }) => label),
    ["#general", "Inbox"],
  );
});

test("history labels cover static and detail routes", () => {
  assert.equal(
    describeHistoryLocation({ pathname: "/", search: {} }, []),
    "Inbox",
  );
  assert.equal(
    describeHistoryLocation({ pathname: "/projects", search: {} }, []),
    "Projects",
  );
  assert.equal(
    describeHistoryLocation(
      { pathname: "/projects/project-a", search: {} },
      [],
    ),
    "Project details",
  );
});

test("an unlabelled route falls back to its pathname, not the inbox", () => {
  assert.equal(
    describeHistoryLocation({ pathname: "/not-labelled-yet", search: {} }, []),
    "/not-labelled-yet",
  );
});
