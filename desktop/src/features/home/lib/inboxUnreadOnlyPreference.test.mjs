import assert from "node:assert/strict";
import test from "node:test";

import {
  INBOX_UNREAD_ONLY_KEY,
  readInboxUnreadOnlyPreference,
  resetInboxUnreadOnlyPreferenceForTests,
  writeInboxUnreadOnlyPreference,
} from "./inboxUnreadOnlyPreference.ts";

function installStorage() {
  const data = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => (data.has(key) ? data.get(key) : null),
      setItem: (key, value) => {
        data.set(key, String(value));
      },
    },
  };
  return data;
}

test("unread-only defaults off and remembers on", () => {
  installStorage();
  resetInboxUnreadOnlyPreferenceForTests();
  assert.equal(readInboxUnreadOnlyPreference(), false);
  writeInboxUnreadOnlyPreference(true);
  resetInboxUnreadOnlyPreferenceForTests();
  assert.equal(readInboxUnreadOnlyPreference(), true);
});

test("unread-only remembers off after it was on", () => {
  const data = installStorage();
  resetInboxUnreadOnlyPreferenceForTests();
  writeInboxUnreadOnlyPreference(true);
  writeInboxUnreadOnlyPreference(false);
  resetInboxUnreadOnlyPreferenceForTests();
  assert.equal(data.get(INBOX_UNREAD_ONLY_KEY), "0");
  assert.equal(readInboxUnreadOnlyPreference(), false);
});

test("in-memory value wins after a write even if storage is empty", () => {
  installStorage();
  resetInboxUnreadOnlyPreferenceForTests();
  writeInboxUnreadOnlyPreference(true);
  assert.equal(readInboxUnreadOnlyPreference(), true);
});
