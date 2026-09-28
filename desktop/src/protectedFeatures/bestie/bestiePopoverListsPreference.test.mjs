import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_POPOVER_LISTS_COLLAPSED_STORAGE_KEY,
  setBestiePopoverListsCollapsed,
  useBestiePopoverListsCollapsed,
} from "./bestiePopoverListsPreference.ts";

test("setBestiePopoverListsCollapsed persists with collapsed default", () => {
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

  setBestiePopoverListsCollapsed(false);
  assert.equal(memory.get(BESTIE_POPOVER_LISTS_COLLAPSED_STORAGE_KEY), "0");
  setBestiePopoverListsCollapsed(true);
  assert.equal(memory.get(BESTIE_POPOVER_LISTS_COLLAPSED_STORAGE_KEY), "1");
  assert.equal(typeof useBestiePopoverListsCollapsed, "function");
});


test("setBestiePopoverListsCollapsed(true) forces collapsed after expand", () => {
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
  setBestiePopoverListsCollapsed(false);
  assert.equal(memory.get(BESTIE_POPOVER_LISTS_COLLAPSED_STORAGE_KEY), "0");
  // Mimic popover open: always force collapsed.
  setBestiePopoverListsCollapsed(true);
  assert.equal(memory.get(BESTIE_POPOVER_LISTS_COLLAPSED_STORAGE_KEY), "1");
});
