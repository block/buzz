import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveFocusedMessageAction,
  shouldClaimComposerTimelineFocus,
} from "./focusedMessageNavigation.ts";

const ALL_AVAILABLE = {
  canReply: true,
  canGoBack: true,
  canReact: true,
  canEdit: true,
  canMarkUnread: true,
  canCopyLink: true,
};

const NONE_AVAILABLE = {
  canReply: false,
  canGoBack: false,
  canReact: false,
  canEdit: false,
  canMarkUnread: false,
  canCopyLink: false,
};

function keyState(overrides = {}) {
  return {
    key: "t",
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    rowOwnsFocus: true,
    overlayOpen: false,
    ...overrides,
  };
}

test("plain arrows move focus, modified arrows are left alone", () => {
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "ArrowUp" }), ALL_AVAILABLE),
    "focus-prev",
  );
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "ArrowDown" }), ALL_AVAILABLE),
    "focus-next",
  );
  for (const modifiers of [
    { shiftKey: true },
    { altKey: true },
    { ctrlKey: true },
    { metaKey: true },
  ]) {
    assert.equal(
      resolveFocusedMessageAction(
        keyState({ key: "ArrowUp", ...modifiers }),
        ALL_AVAILABLE,
      ),
      null,
    );
    assert.equal(
      resolveFocusedMessageAction(
        keyState({ key: "ArrowDown", ...modifiers }),
        ALL_AVAILABLE,
      ),
      null,
    );
  }
});

test("single letters map to actions and respect availability", () => {
  const expected = {
    t: "reply",
    r: "react",
    e: "edit",
    u: "mark-unread",
    l: "copy-link",
  };
  for (const [letter, action] of Object.entries(expected)) {
    assert.equal(
      resolveFocusedMessageAction(keyState({ key: letter }), ALL_AVAILABLE),
      action,
    );
    // Shift folds case: `T` still replies.
    assert.equal(
      resolveFocusedMessageAction(
        keyState({ key: letter.toUpperCase(), shiftKey: true }),
        ALL_AVAILABLE,
      ),
      action,
    );
    assert.equal(
      resolveFocusedMessageAction(keyState({ key: letter }), NONE_AVAILABLE),
      null,
    );
  }
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "q" }), ALL_AVAILABLE),
    null,
  );
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "Enter" }), ALL_AVAILABLE),
    null,
  );
});

test("letters require no Ctrl/Meta/Alt and an unmodified Escape returns to composer", () => {
  for (const modifiers of [
    { ctrlKey: true },
    { metaKey: true },
    { altKey: true },
  ]) {
    assert.equal(
      resolveFocusedMessageAction(
        keyState({ key: "t", ...modifiers }),
        ALL_AVAILABLE,
      ),
      null,
    );
  }
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "Escape" }), NONE_AVAILABLE),
    "to-composer",
  );
  assert.equal(
    resolveFocusedMessageAction(
      keyState({ key: "Escape", ctrlKey: true }),
      ALL_AVAILABLE,
    ),
    null,
  );
  assert.equal(
    resolveFocusedMessageAction(
      keyState({ key: "Escape", shiftKey: true }),
      ALL_AVAILABLE,
    ),
    null,
  );
});

test("IME, nested focus, and open overlays swallow every shortcut", () => {
  for (const guard of [
    { isComposing: true },
    { rowOwnsFocus: false },
    { overlayOpen: true },
  ]) {
    for (const key of ["ArrowUp", "ArrowDown", "ArrowRight", "Escape", "t"]) {
      assert.equal(
        resolveFocusedMessageAction(keyState({ key, ...guard }), ALL_AVAILABLE),
        null,
        `${key} with ${Object.keys(guard)[0]}`,
      );
    }
  }
});

test("ArrowRight replies and ArrowLeft goes back only when available", () => {
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "ArrowRight" }), ALL_AVAILABLE),
    "reply",
  );
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "ArrowRight" }), {
      ...ALL_AVAILABLE,
      canReply: false,
    }),
    null,
  );
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "ArrowLeft" }), ALL_AVAILABLE),
    "go-back",
  );
  assert.equal(
    resolveFocusedMessageAction(keyState({ key: "ArrowLeft" }), {
      ...ALL_AVAILABLE,
      canGoBack: false,
    }),
    null,
  );
  // Modified arrows never navigate (Alt+arrows stay reserved for channels).
  assert.equal(
    resolveFocusedMessageAction(
      keyState({ key: "ArrowRight", altKey: true }),
      ALL_AVAILABLE,
    ),
    null,
  );
  assert.equal(
    resolveFocusedMessageAction(
      keyState({ key: "ArrowLeft", altKey: true }),
      ALL_AVAILABLE,
    ),
    null,
  );
});

function composerKey(overrides = {}) {
  return {
    key: "F6",
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    isComposing: false,
    autocompleteOpen: false,
    overlayOpen: false,
    ...overrides,
  };
}

test("F6 claims timeline focus; modifiers, IME, autocomplete, and overlays decline", () => {
  assert.equal(shouldClaimComposerTimelineFocus(composerKey()), true);
  for (const decline of [
    { key: "F5" },
    { ctrlKey: true },
    { metaKey: true },
    { altKey: true },
    { shiftKey: true },
    { repeat: true },
    { isComposing: true },
    { autocompleteOpen: true },
    { overlayOpen: true },
  ]) {
    assert.equal(
      shouldClaimComposerTimelineFocus(composerKey(decline)),
      false,
      JSON.stringify(decline),
    );
  }
});
