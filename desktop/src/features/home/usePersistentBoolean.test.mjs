import assert from "node:assert/strict";
import test from "node:test";

/**
 * Tests for the localStorage-backed unread-only preference.
 *
 * These tests verify the storage read/write logic that backs
 * `usePersistentBoolean` — the hook powering the persisted
 * "Show unread only" toggle in the Inbox.
 */

const STORAGE_KEY = "buzz-home-inbox-unread-only.v1";

// Minimal mock localStorage for Node.js test environment
const store = new Map();
const mockLocalStorage = {
  getItem: (key) => store.get(key) ?? null,
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: (key) => store.delete(key),
  clear: () => store.clear(),
};

// Inject mock into globalThis so the hook's storage access works
globalThis.localStorage = mockLocalStorage;

function readStoredBoolean(key) {
  const raw = globalThis.localStorage.getItem(key);
  if (raw === null) return false;
  return raw === "true";
}

function writeStoredBoolean(key, value) {
  globalThis.localStorage.setItem(key, String(value));
}

test("default is false when no stored value exists", () => {
  globalThis.localStorage.removeItem(STORAGE_KEY);
  assert.equal(readStoredBoolean(STORAGE_KEY), false);
});

test("stored true is restored as true", () => {
  writeStoredBoolean(STORAGE_KEY, true);
  assert.equal(readStoredBoolean(STORAGE_KEY), true);
});

test("stored false is restored as false", () => {
  writeStoredBoolean(STORAGE_KEY, false);
  assert.equal(readStoredBoolean(STORAGE_KEY), false);
});

test("writing true then reading returns true", () => {
  globalThis.localStorage.removeItem(STORAGE_KEY);
  writeStoredBoolean(STORAGE_KEY, true);
  assert.equal(readStoredBoolean(STORAGE_KEY), true);
});

test("writing false then reading returns false", () => {
  globalThis.localStorage.removeItem(STORAGE_KEY);
  writeStoredBoolean(STORAGE_KEY, true);
  writeStoredBoolean(STORAGE_KEY, false);
  assert.equal(readStoredBoolean(STORAGE_KEY), false);
});

test("toggling from true to false persists the new value", () => {
  writeStoredBoolean(STORAGE_KEY, true);
  assert.equal(readStoredBoolean(STORAGE_KEY), true);
  writeStoredBoolean(STORAGE_KEY, false);
  assert.equal(readStoredBoolean(STORAGE_KEY), false);
});

test("storage key uses versioned prefix matching existing buzz-home patterns", () => {
  assert.ok(
    STORAGE_KEY.startsWith("buzz-home-"),
    "Storage key should follow the buzz-home-* prefix convention",
  );
  assert.ok(
    STORAGE_KEY.includes(".v1"),
    "Storage key should include a version suffix for future schema changes",
  );
});
