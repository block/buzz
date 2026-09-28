import assert from "node:assert/strict";
import test from "node:test";

import {
  addBestieScratchNote,
  bestieScratchSnippet,
  bestieScratchStorageKey,
  deriveBestieScratchTitle,
  emptyBestieScratchState,
  parseBestieScratchState,
  readBestieScratchState,
  removeBestieScratchNote,
  updateBestieScratchNote,
  writeBestieScratchState,
} from "./bestieScratchStorage.ts";

const SCOPE = {
  agentPubkey: "A".repeat(64),
  ownerPubkey: "B".repeat(64),
  relayUrl: "wss://Example.COM/relay",
};

test("storage key is scoped by relay and owner only (no agent)", () => {
  const key = bestieScratchStorageKey(SCOPE);
  assert.match(key, /^buzz-bestie-scratch\.v1:/);
  assert.doesNotMatch(key, /:a{64}$/);
  assert.match(key, /b{64}/);
});

test("deriveBestieScratchTitle and snippet stay short", () => {
  assert.equal(deriveBestieScratchTitle("  First line\nSecond"), "First line");
  assert.equal(deriveBestieScratchTitle("   "), "Untitled");
  const long = "x".repeat(100);
  assert.equal(bestieScratchSnippet(long).endsWith("…"), true);
});

test("parseBestieScratchState accepts valid payloads only", () => {
  assert.equal(parseBestieScratchState(null), null);
  assert.equal(parseBestieScratchState({ version: 2, notes: [] }), null);
  const parsed = parseBestieScratchState({
    version: 1,
    notes: [
      {
        id: "1",
        title: " Idea ",
        body: "body",
        createdAt: 1,
        updatedAt: 1,
        sourceMessageId: null,
      },
      { id: "bad" },
    ],
    processedMessageIds: ["m1", 2],
  });
  assert.equal(parsed.notes.length, 1);
  assert.equal(parsed.notes[0].title, "Idea");
  assert.deepEqual(parsed.processedMessageIds, ["m1"]);
});

test("read/write round-trip through localStorage", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  let state = emptyBestieScratchState();
  state = addBestieScratchNote(state, { body: "Park me", title: "" }, 10);
  assert.equal(state.notes[0].title, "Park me");
  state = updateBestieScratchNote(
    state,
    { body: "Updated body", id: state.notes[0].id, title: "Updated" },
    11,
  );
  writeBestieScratchState(SCOPE, state);
  const loaded = readBestieScratchState(SCOPE);
  assert.equal(loaded.notes.length, 1);
  assert.equal(loaded.notes[0].title, "Updated");
  assert.equal(loaded.notes[0].body, "Updated body");
  const cleared = removeBestieScratchNote(loaded, loaded.notes[0].id);
  assert.equal(cleared.notes.length, 0);
});
