import assert from "node:assert/strict";
import test from "node:test";

import {
  GLOBAL_SHORTCUT_HINT_IDS,
  getShortcutHintsForView,
} from "./keyboard-shortcuts.ts";

test("global anchors surface on an arbitrary view", () => {
  const hints = getShortcutHintsForView("home");
  const ids = hints.map((hint) => hint.id);
  for (const globalId of GLOBAL_SHORTCUT_HINT_IDS) {
    assert.ok(
      ids.includes(globalId),
      `expected global hint "${globalId}" to be present`,
    );
  }
});

test("quick-search is surfaced with resolved key metadata", () => {
  const hints = getShortcutHintsForView("channel");
  const quickSearch = hints.find((hint) => hint.id === "quick-search");
  assert.ok(quickSearch, "quick-search hint must resolve");
  assert.equal(quickSearch.keys, "⌘K");
  assert.equal(quickSearch.keysWindows, "Ctrl+K");
});

test("unknown view falls back to global anchors only", () => {
  const hints = getShortcutHintsForView("does-not-exist");
  const ids = hints.map((hint) => hint.id);
  assert.deepEqual(ids, [...GLOBAL_SHORTCUT_HINT_IDS]);
});

test("hints are de-duplicated and every id resolves to a known shortcut", () => {
  const hints = getShortcutHintsForView("channel");
  const ids = hints.map((hint) => hint.id);
  assert.equal(
    new Set(ids).size,
    ids.length,
    "hint list must not contain duplicate ids",
  );
  for (const hint of hints) {
    assert.ok(hint.label.length > 0, `hint "${hint.id}" must have a label`);
  }
});
