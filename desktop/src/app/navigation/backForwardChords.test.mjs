import assert from "node:assert/strict";
import test from "node:test";

import { matchBackForwardChord } from "./backForwardChords.ts";

function chord(overrides = {}) {
  return {
    altKey: false,
    code: "",
    ctrlKey: false,
    key: "",
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

// ── macOS: ⌘[ / ⌘] ───────────────────────────────────────────────────────────

test("mac: ⌘[ matches back", () => {
  assert.equal(
    matchBackForwardChord(chord({ key: "[", metaKey: true }), true),
    "back",
  );
});

test("mac: ⌘] matches forward", () => {
  assert.equal(
    matchBackForwardChord(chord({ key: "]", metaKey: true }), true),
    "forward",
  );
});

test("mac: matches by code for non-US layouts", () => {
  assert.equal(
    matchBackForwardChord(
      chord({ code: "BracketLeft", key: "Dead", metaKey: true }),
      true,
    ),
    "back",
  );
  assert.equal(
    matchBackForwardChord(
      chord({ code: "BracketRight", key: "Dead", metaKey: true }),
      true,
    ),
    "forward",
  );
});

test("mac: requires meta", () => {
  assert.equal(matchBackForwardChord(chord({ key: "[" }), true), null);
});

test("mac: rejects extra modifiers", () => {
  for (const extra of [
    { altKey: true },
    { ctrlKey: true },
    { shiftKey: true },
  ]) {
    assert.equal(
      matchBackForwardChord(chord({ key: "[", metaKey: true, ...extra }), true),
      null,
    );
  }
});

test("mac: Alt+arrows do not match (that is the win/linux chord)", () => {
  assert.equal(
    matchBackForwardChord(chord({ altKey: true, key: "ArrowLeft" }), true),
    null,
  );
});

test("mac: ⌘←/⌘→ never match — they are line start/end in text editing", () => {
  // Deliberately unbound: editable targets must keep receiving ⌘←/⌘→ so
  // line-start/line-end editing still works. Only ⌘[ / ⌘] navigate.
  assert.equal(
    matchBackForwardChord(chord({ key: "ArrowLeft", metaKey: true }), true),
    null,
  );
  assert.equal(
    matchBackForwardChord(chord({ key: "ArrowRight", metaKey: true }), true),
    null,
  );
});

// ── Windows/Linux: Alt+← / Alt+→ ─────────────────────────────────────────────

test("win/linux: Alt+ArrowLeft matches back", () => {
  assert.equal(
    matchBackForwardChord(chord({ altKey: true, key: "ArrowLeft" }), false),
    "back",
  );
});

test("win/linux: Alt+ArrowRight matches forward", () => {
  assert.equal(
    matchBackForwardChord(chord({ altKey: true, key: "ArrowRight" }), false),
    "forward",
  );
});

test("win/linux: requires alt", () => {
  assert.equal(matchBackForwardChord(chord({ key: "ArrowLeft" }), false), null);
});

test("win/linux: rejects extra modifiers", () => {
  for (const extra of [
    { ctrlKey: true },
    { metaKey: true },
    { shiftKey: true },
  ]) {
    assert.equal(
      matchBackForwardChord(
        chord({ altKey: true, key: "ArrowLeft", ...extra }),
        false,
      ),
      null,
    );
  }
});

test("win/linux: ⌘[ does not match (that is the mac chord)", () => {
  assert.equal(
    matchBackForwardChord(chord({ key: "[", metaKey: true }), false),
    null,
  );
});

// ── Non-chord keys never match ────────────────────────────────────────────────

test("plain bracket / arrow keys without the platform modifier never match", () => {
  for (const isMac of [true, false]) {
    for (const key of ["[", "]", "ArrowLeft", "ArrowRight", "a", "Enter"]) {
      assert.equal(matchBackForwardChord(chord({ key }), isMac), null);
    }
  }
});
