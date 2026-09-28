import assert from "node:assert/strict";
import test from "node:test";

import {
  beginBestieCoffeeRun,
  bestieCoffeeStorageKey,
  completeBestieCoffeeRun,
  emptyBestieCoffeeState,
  isBestieCoffeeBrewing,
  parseBestieCoffeeState,
  readBestieCoffeeState,
  removeBestieCoffeeEntry,
  writeBestieCoffeeState,
} from "./bestieCoffeeStorage.ts";
import { localDayKey } from "./bestieCoffeeSchedule.ts";

const SCOPE = {
  agentPubkey: "a".repeat(64),
  ownerPubkey: "b".repeat(64),
  relayUrl: "wss://relay.test",
};

function memoryWindow() {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      key: (index) => [...memory.keys()][index] ?? null,
      get length() {
        return memory.size;
      },
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };
  return memory;
}

test("brief+expand storage: complete appends entry and clears pending", () => {
  let state = emptyBestieCoffeeState();
  const begun = beginBestieCoffeeRun(state, "scheduled", 1_000);
  assert.ok(begun);
  assert.equal(isBestieCoffeeBrewing(begun), true);
  assert.equal(begun.lastScheduledDayKey, localDayKey(1_000));
  assert.equal(beginBestieCoffeeRun(begun, "brew", 1_001), null);

  state = completeBestieCoffeeRun(
    begun,
    {
      brief: "",
      fullOutput: "Morning looks calm. Details below.",
      source: "scheduled",
    },
    1_100,
  );
  assert.equal(state.pendingRun, null);
  assert.equal(state.entries.length, 1);
  assert.equal(state.entries[0].brief, "Morning looks calm.");
  assert.match(state.entries[0].fullOutput, /Details below/);
  assert.equal(state.lastScheduledDayKey, localDayKey(1_100));
});

test("scheduled begin claims the day so remount cannot re-fire", () => {
  const now = Math.floor(new Date(2026, 8, 28, 8, 5, 0).getTime() / 1000);
  let state = emptyBestieCoffeeState();
  const begun = beginBestieCoffeeRun(state, "scheduled", now);
  assert.ok(begun);
  assert.equal(begun.lastScheduledDayKey, localDayKey(now));
  // Simulate stale-pending clear on reload (pending dropped, day kept).
  state = { ...begun, pendingRun: null };
  assert.equal(beginBestieCoffeeRun(state, "scheduled", now + 60), null);
  // Brew still allowed when not brewing.
  const brew = beginBestieCoffeeRun(state, "brew", now + 60);
  assert.ok(brew);
  assert.equal(brew.lastScheduledDayKey, localDayKey(now));
});

test("remove coffee entry remembers trigger and writes owner key durably", () => {
  const memory = memoryWindow();
  let state = emptyBestieCoffeeState();
  state = completeBestieCoffeeRun(
    state,
    {
      brief: "Brief",
      fullOutput: "Full brew output",
      replyMessageId: "reply-1",
      source: "brew",
      triggerMessageId: "trigger-1",
    },
    1_700_000_500,
  );
  assert.equal(state.entries.length, 1);
  state = removeBestieCoffeeEntry(state, state.entries[0].id);
  assert.equal(state.entries.length, 0);
  assert.deepEqual(state.forgottenTriggerIds, ["trigger-1"]);
  writeBestieCoffeeState(SCOPE, state);

  const ownerKey = bestieCoffeeStorageKey(SCOPE);
  assert.ok(memory.has(ownerKey));
  const stored = JSON.parse(memory.get(ownerKey));
  assert.equal(stored.entries.length, 0);
  assert.deepEqual(stored.forgottenTriggerIds, ["trigger-1"]);

  const legacyKey = `${ownerKey}:${SCOPE.agentPubkey.toLowerCase()}`;
  memory.set(
    legacyKey,
    JSON.stringify({
      version: 1,
      entries: [
        {
          id: "zombie",
          ranAt: 1,
          brief: "zombie",
          fullOutput: "should not return",
          source: "brew",
          triggerMessageId: "trigger-1",
          replyMessageId: "r",
        },
      ],
      forgottenTriggerIds: [],
      lastScheduledDayKey: null,
      pendingRun: null,
      prefs: { hour: 8, minute: 0 },
    }),
  );
  writeBestieCoffeeState(SCOPE, state);
  assert.equal(memory.has(legacyKey), false);
  const loaded = readBestieCoffeeState(SCOPE);
  assert.equal(loaded.entries.length, 0);
  assert.deepEqual(loaded.forgottenTriggerIds, ["trigger-1"]);
});

test("parseBestieCoffeeState keeps forgottenTriggerIds", () => {
  const parsed = parseBestieCoffeeState({
    version: 1,
    entries: [],
    forgottenTriggerIds: ["t1", "t1", "", 3],
    prefs: { hour: 8, minute: 0 },
    lastScheduledDayKey: null,
    pendingRun: null,
  });
  assert.deepEqual(parsed.forgottenTriggerIds, ["t1"]);
});
