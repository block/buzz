import assert from "node:assert/strict";
import test from "node:test";

import {
  INBOX_UNREAD_ONLY_STORAGE_KEY,
  readInboxUnreadOnlyPreference,
  writeInboxUnreadOnlyPreference,
} from "./inboxUnreadOnlyPreference.ts";

function memoryStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return map.has(key) ? map.get(key) : null;
    },
    setItem(key, value) {
      map.set(key, String(value));
    },
    store: map,
  };
}

test("readInboxUnreadOnlyPreference defaults to false when unset", () => {
  const storage = memoryStorage();
  assert.equal(readInboxUnreadOnlyPreference(storage), false);
});

test("readInboxUnreadOnlyPreference returns true for true/1", () => {
  assert.equal(
    readInboxUnreadOnlyPreference(
      memoryStorage({ [INBOX_UNREAD_ONLY_STORAGE_KEY]: "true" }),
    ),
    true,
  );
  assert.equal(
    readInboxUnreadOnlyPreference(
      memoryStorage({ [INBOX_UNREAD_ONLY_STORAGE_KEY]: "1" }),
    ),
    true,
  );
});

test("readInboxUnreadOnlyPreference returns false for other values", () => {
  assert.equal(
    readInboxUnreadOnlyPreference(
      memoryStorage({ [INBOX_UNREAD_ONLY_STORAGE_KEY]: "false" }),
    ),
    false,
  );
  assert.equal(
    readInboxUnreadOnlyPreference(
      memoryStorage({ [INBOX_UNREAD_ONLY_STORAGE_KEY]: "yes" }),
    ),
    false,
  );
});

test("writeInboxUnreadOnlyPreference persists and round-trips", () => {
  const storage = memoryStorage();
  writeInboxUnreadOnlyPreference(true, storage);
  assert.equal(storage.getItem(INBOX_UNREAD_ONLY_STORAGE_KEY), "true");
  assert.equal(readInboxUnreadOnlyPreference(storage), true);

  writeInboxUnreadOnlyPreference(false, storage);
  assert.equal(storage.getItem(INBOX_UNREAD_ONLY_STORAGE_KEY), "false");
  assert.equal(readInboxUnreadOnlyPreference(storage), false);
});

test("readInboxUnreadOnlyPreference returns false when storage is null", () => {
  assert.equal(readInboxUnreadOnlyPreference(null), false);
});
