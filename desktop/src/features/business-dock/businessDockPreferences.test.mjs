import assert from "node:assert/strict";
import test from "node:test";

import {
  BUSINESS_DOCK_PREFERENCES_KEY,
  readBusinessDockPreferences,
  saveBusinessDockPreferences,
} from "./businessDockPreferences.ts";

test("business dock preferences default and persist locally", () => {
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };
  assert.deepEqual(readBusinessDockPreferences(storage), {
    followConversation: true,
    pinned: false,
  });
  saveBusinessDockPreferences(storage, {
    followConversation: false,
    pinned: true,
  });
  assert.deepEqual(JSON.parse(values.get(BUSINESS_DOCK_PREFERENCES_KEY)), {
    followConversation: false,
    pinned: true,
  });
  assert.deepEqual(readBusinessDockPreferences(storage), {
    followConversation: false,
    pinned: true,
  });
});

test("malformed or unavailable storage fails to defaults", () => {
  assert.deepEqual(readBusinessDockPreferences({ getItem: () => "{" }), {
    followConversation: true,
    pinned: false,
  });
  assert.doesNotThrow(() =>
    saveBusinessDockPreferences(
      {
        getItem: () => null,
        setItem: () => {
          throw new Error("quota");
        },
      },
      { followConversation: true, pinned: false },
    ),
  );
});
