import assert from "node:assert/strict";
import test from "node:test";

import { hasPrimaryShortcutModifier } from "./platform.ts";

const originalNavigator = Object.getOwnPropertyDescriptor(
  globalThis,
  "navigator",
);

function withPlatform(platform, userAgent, run) {
  Object.defineProperty(globalThis, "navigator", {
    value: { platform, userAgent: userAgent ?? "" },
    configurable: true,
    writable: true,
  });
  try {
    run();
  } finally {
    if (originalNavigator) {
      Object.defineProperty(globalThis, "navigator", originalNavigator);
    } else {
      delete globalThis.navigator;
    }
  }
}

const mods = (overrides) => ({
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...overrides,
});

// ── macOS: Command is the primary modifier, Control is NOT ───────────────────

test("macOS: Cmd (meta) is the primary modifier", () => {
  withPlatform("MacIntel", "", () => {
    assert.equal(hasPrimaryShortcutModifier(mods({ metaKey: true })), true);
  });
});

test("macOS: Ctrl is rejected (it is a right-click / Emacs binding there)", () => {
  withPlatform("MacIntel", "", () => {
    assert.equal(hasPrimaryShortcutModifier(mods({ ctrlKey: true })), false);
  });
});

test("macOS: Cmd+Ctrl together is rejected", () => {
  withPlatform("MacIntel", "", () => {
    assert.equal(
      hasPrimaryShortcutModifier(mods({ metaKey: true, ctrlKey: true })),
      false,
    );
  });
});

test("macOS: no modifier is not the primary modifier", () => {
  withPlatform("MacIntel", "", () => {
    assert.equal(hasPrimaryShortcutModifier(mods()), false);
  });
});

// ── Windows/Linux: Control is the primary modifier, Meta is NOT ──────────────

test("Windows: Ctrl is the primary modifier", () => {
  withPlatform("Win32", "", () => {
    assert.equal(hasPrimaryShortcutModifier(mods({ ctrlKey: true })), true);
  });
});

test("Windows: Meta (Windows key) is not the primary modifier", () => {
  withPlatform("Win32", "", () => {
    assert.equal(hasPrimaryShortcutModifier(mods({ metaKey: true })), false);
  });
});

test("Linux: Ctrl is the primary modifier, Meta+Ctrl is rejected", () => {
  withPlatform("Linux x86_64", "X11; Linux x86_64", () => {
    assert.equal(hasPrimaryShortcutModifier(mods({ ctrlKey: true })), true);
    assert.equal(
      hasPrimaryShortcutModifier(mods({ ctrlKey: true, metaKey: true })),
      false,
    );
  });
});
