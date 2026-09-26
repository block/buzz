import assert from "node:assert/strict";
import test from "node:test";

import {
  loadPersonalPinnedSites,
  parsePersonalPinnedSitesBlob,
  personalPinnedSitesStorageKey,
  removePersonalPin,
  seedWayfinderIfNeeded,
  upsertPersonalPin,
} from "./personalPins.ts";
import { WAYFINDER_PIN, WAYFINDER_PIN_ID } from "./types.ts";

const memory = new Map();

function installStorage() {
  memory.clear();
  const localStorage = {
    getItem(key) {
      return memory.has(key) ? memory.get(key) : null;
    },
    setItem(key, value) {
      memory.set(key, value);
    },
    removeItem(key) {
      memory.delete(key);
    },
  };
  globalThis.localStorage = localStorage;
  globalThis.window = { localStorage };
}

test("seeds Wayfinder once on an empty store", () => {
  const seeded = seedWayfinderIfNeeded({
    version: 1,
    wayfinderSeeded: false,
    pins: [],
  });
  assert.equal(seeded.wayfinderSeeded, true);
  assert.equal(seeded.pins.length, 1);
  assert.equal(seeded.pins[0].id, WAYFINDER_PIN_ID);
  assert.equal(seeded.pins[0].name, "Wayfinder");
  assert.equal(seeded.pins[0].url, WAYFINDER_PIN.url);
});

test("does not recreate Wayfinder after it is deleted", () => {
  const afterDelete = removePersonalPin(
    {
      version: 1,
      wayfinderSeeded: true,
      pins: [{ ...WAYFINDER_PIN }],
    },
    WAYFINDER_PIN_ID,
  );
  const laterLaunch = seedWayfinderIfNeeded(afterDelete);
  assert.equal(laterLaunch.pins.length, 0);
  assert.equal(laterLaunch.wayfinderSeeded, true);
});

test("parsePersonalPinnedSitesBlob rejects http urls", () => {
  const parsed = parsePersonalPinnedSitesBlob({
    version: 1,
    wayfinderSeeded: true,
    pins: [
      {
        id: "bad",
        name: "Bad",
        url: "http://example.com",
        icon: "globe",
      },
    ],
  });
  assert.deepEqual(parsed?.pins, []);
});

test("upsertPersonalPin replaces an existing id", () => {
  const next = upsertPersonalPin(
    {
      version: 1,
      wayfinderSeeded: true,
      pins: [{ ...WAYFINDER_PIN }],
    },
    { ...WAYFINDER_PIN, name: "Maps" },
  );
  assert.equal(next.pins.length, 1);
  assert.equal(next.pins[0].name, "Maps");
});

test("loadPersonalPinnedSites persists the seed flag via storage key", () => {
  installStorage();
  const key = personalPinnedSitesStorageKey("abc", "wss://relay.example.com/");
  assert.match(key, /buzz-pinned-sites\.v1:abc:/);
  const first = loadPersonalPinnedSites("abc", "wss://relay.example.com");
  assert.equal(first.pins[0].id, WAYFINDER_PIN_ID);
});

test("openMatchingLinks defaults to true when missing from stored blob", () => {
  const parsed = parsePersonalPinnedSitesBlob({
    version: 1,
    wayfinderSeeded: true,
    pins: [
      {
        id: "legacy",
        name: "Legacy",
        url: "https://example.com",
        icon: "globe",
      },
    ],
  });
  assert.equal(parsed?.pins.length, 1);
  assert.equal(parsed?.pins[0].openMatchingLinks, true);
});

test("openMatchingLinks false is preserved", () => {
  const parsed = parsePersonalPinnedSitesBlob({
    version: 1,
    wayfinderSeeded: true,
    pins: [
      {
        id: "off",
        name: "Off",
        url: "https://example.com",
        icon: "globe",
        openMatchingLinks: false,
      },
    ],
  });
  assert.equal(parsed?.pins[0].openMatchingLinks, false);
});
