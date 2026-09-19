import assert from "node:assert/strict";
import test from "node:test";

import { isCloseWindowShortcut } from "./useCloseWindowShortcut.ts";

function chord(overrides = {}) {
  return {
    altKey: false,
    code: "KeyW",
    ctrlKey: false,
    defaultPrevented: false,
    isComposing: false,
    metaKey: true,
    repeat: false,
    shiftKey: false,
    ...overrides,
  };
}

test("Cmd+W closes a window on macOS", () => {
  assert.equal(isCloseWindowShortcut(chord(), true), true);
});

test("the close-window shortcut rejects other platforms and modified chords", () => {
  assert.equal(isCloseWindowShortcut(chord(), false), false);
  assert.equal(isCloseWindowShortcut(chord({ metaKey: false }), true), false);
  assert.equal(isCloseWindowShortcut(chord({ ctrlKey: true }), true), false);
  assert.equal(isCloseWindowShortcut(chord({ altKey: true }), true), false);
  assert.equal(isCloseWindowShortcut(chord({ shiftKey: true }), true), false);
  assert.equal(isCloseWindowShortcut(chord({ code: "KeyQ" }), true), false);
});

test("handled, composing, and repeated events are left alone", () => {
  assert.equal(
    isCloseWindowShortcut(chord({ defaultPrevented: true }), true),
    false,
  );
  assert.equal(
    isCloseWindowShortcut(chord({ isComposing: true }), true),
    false,
  );
  assert.equal(isCloseWindowShortcut(chord({ repeat: true }), true), false);
});
