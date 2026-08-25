import assert from "node:assert/strict";
import test from "node:test";

import { isBusinessDockShortcut } from "./businessDockShortcut.ts";

const keyboardEvent = (overrides = {}) => ({
  altKey: false,
  ctrlKey: false,
  key: "b",
  metaKey: false,
  shiftKey: false,
  ...overrides,
});

test("Business Dock shortcut accepts macOS and cross-platform chords", () => {
  assert.equal(
    isBusinessDockShortcut(keyboardEvent({ metaKey: true, shiftKey: true })),
    true,
  );
  assert.equal(
    isBusinessDockShortcut(keyboardEvent({ ctrlKey: true, shiftKey: true })),
    true,
  );
});

test("Business Dock shortcut rejects incomplete or modified chords", () => {
  assert.equal(isBusinessDockShortcut(keyboardEvent({ metaKey: true })), false);
  assert.equal(
    isBusinessDockShortcut(
      keyboardEvent({ altKey: true, metaKey: true, shiftKey: true }),
    ),
    false,
  );
  assert.equal(
    isBusinessDockShortcut(
      keyboardEvent({ key: "x", metaKey: true, shiftKey: true }),
    ),
    false,
  );
});
