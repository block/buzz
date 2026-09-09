import assert from "node:assert/strict";
import test from "node:test";

import {
  matchModShiftChord,
  matchUnreadConversationChord,
} from "./useUnreadConversationShortcuts.ts";

function chord(overrides = {}) {
  return {
    altKey: false,
    code: "",
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

const macUp = () => chord({ altKey: true, shiftKey: true, code: "ArrowUp" });
const macDown = () =>
  chord({ altKey: true, shiftKey: true, code: "ArrowDown" });
const winUp = () =>
  chord({ altKey: true, shiftKey: true, ctrlKey: true, code: "ArrowUp" });
const winDown = () =>
  chord({ altKey: true, shiftKey: true, ctrlKey: true, code: "ArrowDown" });

test("macOS Alt+Shift+Arrows step to the adjacent unread conversation", () => {
  assert.equal(matchUnreadConversationChord(macUp(), true), "previous");
  assert.equal(matchUnreadConversationChord(macDown(), true), "next");
});

test("Windows/Linux needs Ctrl on the unread step chord", () => {
  assert.equal(matchUnreadConversationChord(winUp(), false), "previous");
  assert.equal(matchUnreadConversationChord(winDown(), false), "next");
  assert.equal(matchUnreadConversationChord(macUp(), false), null);
  assert.equal(matchUnreadConversationChord(winUp(), true), null);
});

test("the unread step chord rejects ordinary navigation and modified chords", () => {
  assert.equal(
    matchUnreadConversationChord(
      chord({ altKey: true, code: "ArrowUp" }),
      true,
    ),
    null,
  );
  assert.equal(
    matchUnreadConversationChord(
      chord({ altKey: true, shiftKey: true, code: "ArrowUp" }),
      false,
    ),
    null,
  );
  assert.equal(
    matchUnreadConversationChord(
      chord({ shiftKey: true, code: "ArrowUp" }),
      true,
    ),
    null,
  );
  assert.equal(
    matchUnreadConversationChord({ ...macUp(), metaKey: true }, true),
    null,
  );
});

test("Mod+Shift+T opens threads without claiming plain terminal chords", () => {
  assert.equal(
    matchModShiftChord(
      chord({ metaKey: true, shiftKey: true, code: "KeyT" }),
      "KeyT",
      true,
    ),
    true,
  );
  assert.equal(
    matchModShiftChord(
      chord({ ctrlKey: true, shiftKey: true, code: "KeyT" }),
      "KeyT",
      false,
    ),
    true,
  );
  assert.equal(
    matchModShiftChord(chord({ metaKey: true, code: "KeyT" }), "KeyT", true),
    false,
  );
  assert.equal(
    matchModShiftChord(chord({ ctrlKey: true, code: "KeyT" }), "KeyT", false),
    false,
  );
});

test("Mod+Shift+J jumps to unread while plain Mod+J stays terminal", () => {
  assert.equal(
    matchModShiftChord(
      chord({ metaKey: true, shiftKey: true, code: "KeyJ" }),
      "KeyJ",
      true,
    ),
    true,
  );
  assert.equal(
    matchModShiftChord(
      chord({ ctrlKey: true, shiftKey: true, code: "KeyJ" }),
      "KeyJ",
      false,
    ),
    true,
  );
  assert.equal(
    matchModShiftChord(chord({ metaKey: true, code: "KeyJ" }), "KeyJ", true),
    false,
  );
  assert.equal(
    matchModShiftChord(chord({ ctrlKey: true, code: "KeyJ" }), "KeyJ", false),
    false,
  );
});

test("Mod+Shift chords reject cross-platform and alt-modified variants", () => {
  assert.equal(
    matchModShiftChord(
      chord({ ctrlKey: true, shiftKey: true, code: "KeyT" }),
      "KeyT",
      true,
    ),
    false,
  );
  assert.equal(
    matchModShiftChord(
      chord({ metaKey: true, shiftKey: true, code: "KeyT" }),
      "KeyT",
      false,
    ),
    false,
  );
  assert.equal(
    matchModShiftChord(
      chord({ metaKey: true, shiftKey: true, altKey: true, code: "KeyT" }),
      "KeyT",
      true,
    ),
    false,
  );
  assert.equal(
    matchModShiftChord(
      chord({ metaKey: true, shiftKey: true, code: "KeyK" }),
      "KeyT",
      true,
    ),
    false,
  );
});
