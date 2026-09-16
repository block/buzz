import assert from "node:assert/strict";
import test from "node:test";

import {
  INITIAL_HANDOFF_STATE,
  accumulateScrollLines,
  encodePaste,
  encodeTerminalKey,
  matchTabChord,
  reduceHandoff,
  stepSession,
} from "./terminalState.ts";

test("ownership changes on completed chord, never key-down or repeat", () => {
  const down = reduceHandoff(INITIAL_HANDOFF_STATE, {
    type: "chord-down",
    repeat: false,
  });
  assert.equal(down.state.owner, "buzz");
  assert.equal(down.toggled, false);
  assert.equal(
    reduceHandoff(down.state, { type: "chord-up" }).state.owner,
    "terminal",
  );
  assert.deepEqual(
    reduceHandoff(INITIAL_HANDOFF_STATE, {
      type: "chord-down",
      repeat: true,
    }),
    { state: INITIAL_HANDOFF_STATE, toggled: false },
  );
});

test("composition and focus loss cancel an armed chord", () => {
  const armed = reduceHandoff(INITIAL_HANDOFF_STATE, {
    type: "chord-down",
    repeat: false,
  }).state;
  const composing = reduceHandoff(armed, { type: "composition-start" }).state;
  assert.equal(reduceHandoff(composing, { type: "chord-up" }).toggled, false);
  const blurred = reduceHandoff(armed, { type: "focus-lost" }).state;
  assert.equal(reduceHandoff(blurred, { type: "chord-up" }).toggled, false);
});

test("terminal key encoding covers control, navigation, and alt prefixes", () => {
  assert.equal(
    encodeTerminalKey({
      key: "c",
      ctrlKey: true,
      altKey: false,
      metaKey: false,
    }),
    "\u0003",
  );
  assert.equal(
    encodeTerminalKey({
      key: "ArrowUp",
      ctrlKey: false,
      altKey: false,
      metaKey: false,
    }),
    "\u001b[A",
  );
  assert.equal(
    encodeTerminalKey({
      key: "x",
      ctrlKey: false,
      altKey: true,
      metaKey: false,
    }),
    "\u001bx",
  );
  assert.equal(
    encodeTerminalKey({
      key: "x",
      ctrlKey: false,
      altKey: false,
      metaKey: true,
    }),
    null,
  );
});

test("bracketed paste wraps the entire payload exactly once", () => {
  assert.equal(encodePaste("a\nb", false), "a\nb");
  assert.equal(encodePaste("a\nb", true), "\u001b[200~a\nb\u001b[201~");
});

test("pixel scrolling retains fractional lines in both directions", () => {
  const state = { remainderPx: 0 };
  let result = accumulateScrollLines(state, 3, 10);
  assert.equal(result.lines, 0);
  result = accumulateScrollLines(result.state, 8, 10);
  assert.equal(result.lines, 1);
  assert.equal(result.state.remainderPx, 1);
  result = accumulateScrollLines(result.state, -12, 10);
  assert.equal(result.lines, -1);
  assert.equal(result.state.remainderPx, -1);
});

function chord(overrides) {
  return {
    altKey: false,
    code: "KeyT",
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

test("tab chords match the platform's primary modifier only", () => {
  assert.equal(matchTabChord(chord({ metaKey: true }), true), "new");
  assert.equal(
    matchTabChord(chord({ code: "KeyW", metaKey: true }), true),
    "close",
  );
  assert.equal(
    matchTabChord(
      chord({ code: "ArrowLeft", metaKey: true, shiftKey: true }),
      true,
    ),
    "previous",
  );
  assert.equal(
    matchTabChord(
      chord({ code: "ArrowRight", metaKey: true, shiftKey: true }),
      true,
    ),
    "next",
  );

  // The whole reason the matcher takes `isMac`: on macOS ^W is werase and ^T
  // transposes. Claiming them here would swallow them before the textarea
  // could encode them for the PTY.
  assert.equal(matchTabChord(chord({ ctrlKey: true }), true), null);
  assert.equal(
    matchTabChord(chord({ code: "KeyW", ctrlKey: true }), true),
    null,
  );
  assert.equal(
    matchTabChord(chord({ ctrlKey: true, metaKey: true }), true),
    null,
    "Control must veto even alongside Command",
  );

  // Non-mac mirrors it: Ctrl is primary, Meta is not.
  assert.equal(matchTabChord(chord({ ctrlKey: true }), false), "new");
  assert.equal(matchTabChord(chord({ metaKey: true }), false), null);

  // Unmodified and Alt-modified keys stay with the terminal.
  assert.equal(matchTabChord(chord({}), true), null);
  assert.equal(
    matchTabChord(chord({ altKey: true, metaKey: true }), true),
    null,
  );
  // Arrows without Shift are cursor keys, not tab switches.
  assert.equal(
    matchTabChord(chord({ code: "ArrowLeft", metaKey: true }), true),
    null,
  );
  // Shift+Cmd+T/W are not chords we claim.
  assert.equal(
    matchTabChord(chord({ metaKey: true, shiftKey: true }), true),
    null,
  );
});

test("tab stepping wraps and skips tabs whose select button is disabled", () => {
  const three = [
    { active: false, closing: false, id: "a" },
    { active: true, closing: false, id: "b" },
    { active: false, closing: false, id: "c" },
  ];
  assert.equal(stepSession(three, 1), "c");
  assert.equal(stepSession(three, -1), "a");

  // Wrap at both ends.
  const first = [
    { active: true, closing: false, id: "a" },
    { active: false, closing: false, id: "b" },
  ];
  assert.equal(stepSession(first, -1), "b");
  assert.equal(stepSession(first, 1), "b");

  // A closing tab is disabled in the tab bar, so the keyboard skips past it.
  assert.equal(
    stepSession(
      [
        { active: true, closing: false, id: "a" },
        { active: false, closing: true, id: "b" },
        { active: false, closing: false, id: "c" },
      ],
      1,
    ),
    "c",
  );

  // Nowhere to go: single tab, and all-others-closing.
  assert.equal(
    stepSession([{ active: true, closing: false, id: "a" }], 1),
    null,
  );
  assert.equal(
    stepSession(
      [
        { active: true, closing: false, id: "a" },
        { active: false, closing: true, id: "b" },
      ],
      1,
    ),
    null,
  );
  assert.equal(stepSession([], 1), null);
});
