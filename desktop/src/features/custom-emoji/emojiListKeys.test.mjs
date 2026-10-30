import assert from "node:assert/strict";
import test from "node:test";

import { emojiListKeyAction } from "./emojiListKeys.ts";

const press = (key, mods = {}) => ({
  key,
  shiftKey: false,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  ...mods,
});

test("list keys cover every modality and Shift+Space never acts like Space", () => {
  const composer = { tabSelects: true };
  const panel = { tabSelects: false };
  const rows = [
    [press("ArrowDown"), composer, "next"],
    [press("ArrowUp"), panel, "prev"],
    [press("Enter"), composer, "select"],
    [press("Enter"), panel, "select"],
    [press("Enter", { shiftKey: true }), composer, null],
    [press("Enter", { metaKey: true }), panel, null],
    [press("Tab"), composer, "select"],
    [press("Tab"), panel, null],
    [press("Tab", { shiftKey: true }), composer, null],
    [press("Escape"), panel, "close"],
    [press(" "), composer, null],
    [press(" ", { shiftKey: true }), composer, null],
    [press(" ", { shiftKey: true }), panel, null],
    [press("a"), panel, null],
  ];
  for (const [event, options, expected] of rows) {
    assert.equal(
      emojiListKeyAction(event, options),
      expected,
      `${event.key} shift=${event.shiftKey} meta=${event.metaKey} tab=${options.tabSelects}`,
    );
  }
});
