import assert from "node:assert/strict";
import { test } from "node:test";

import { nameError, withMarkdownExtension } from "./useVaultMutations.ts";

test("adds a markdown extension only when one is missing", () => {
  assert.equal(withMarkdownExtension("Meeting notes"), "Meeting notes.md");
  assert.equal(withMarkdownExtension("Meeting notes.md"), "Meeting notes.md");
  assert.equal(withMarkdownExtension("legacy.markdown"), "legacy.markdown");
  assert.equal(withMarkdownExtension("Notes.MD"), "Notes.MD");
  // A dot that is not an extension must not defeat the check.
  assert.equal(withMarkdownExtension("v1.2 plan"), "v1.2 plan.md");
});

test("accepts ordinary names", () => {
  for (const name of ["Meeting notes", "v1.2 plan", "Ideas!", "a"]) {
    assert.equal(nameError(name), null, name);
  }
});

test("rejects empty and whitespace-only names", () => {
  assert.match(nameError(""), /Enter a name/);
  assert.match(nameError("   "), /Enter a name/);
});

test("rejects path separators rather than creating nested folders", () => {
  assert.match(nameError("Notes/nested"), /slashes/);
  assert.match(nameError("Notes\\nested"), /slashes/);
});

test("rejects leading dots, which the tree would hide", () => {
  // Creating `.private` would succeed on disk and then vanish from the tree,
  // which reads as data loss.
  assert.match(nameError(".private"), /dot/);
  assert.match(nameError("."), /dot/);
  assert.match(nameError(".."), /dot/);
});
