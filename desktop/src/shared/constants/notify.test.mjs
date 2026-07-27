import assert from "node:assert/strict";
import test from "node:test";

import { isNotifyMode, notifyModeFromTags } from "./notify.ts";

test("only the lowercase wire spellings are notify modes", () => {
  assert.equal(isNotifyMode("channel"), true);
  assert.equal(isNotifyMode("here"), true);
  assert.equal(isNotifyMode("Channel"), false);
  assert.equal(isNotifyMode("everyone"), false);
});

test("notifyModeFromTags reads the first well-formed marker", () => {
  assert.equal(notifyModeFromTags([]), null);
  assert.equal(notifyModeFromTags([["notify", "here"]]), "here");
  assert.equal(
    notifyModeFromTags([
      ["notify", "channel"],
      ["notify", "here"],
    ]),
    "channel",
  );
});

test("notifyModeFromTags degrades a malformed marker to no mention", () => {
  assert.equal(notifyModeFromTags([["notify"]]), null);
  assert.equal(notifyModeFromTags([["notify", "EVERYONE"]]), null);
});
