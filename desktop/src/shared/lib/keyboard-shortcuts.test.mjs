import assert from "node:assert/strict";
import test from "node:test";

import { getShortcutsByCategory } from "./keyboard-shortcuts.ts";

function shortcutIds(categories) {
  return [...categories.values()].flat().map((shortcut) => shortcut.id);
}

test("feature-owned shortcuts stay out of the default catalog", () => {
  assert.equal(
    shortcutIds(getShortcutsByCategory()).includes("open-bestie"),
    false,
  );
});

test("the Bestie shortcut appears only when its experiment is effective", () => {
  assert.equal(
    shortcutIds(getShortcutsByCategory(new Set(["bestie"]))).includes(
      "open-bestie",
    ),
    true,
  );
});
