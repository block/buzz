import assert from "node:assert/strict";
import test from "node:test";

import { isObsidianOpenLink } from "./obsidianLink.ts";

test("accepts Obsidian open links for vault-relative and absolute targets", () => {
  assert.equal(
    isObsidianOpenLink(
      "obsidian://open?vault=holistics-digest&file=agent-cowork%2Fprojects%2Fanfra%2FANFRA_SETUP_END_TO_END.canvas",
    ),
    true,
  );
  assert.equal(isObsidianOpenLink("obsidian://open?file=README.md"), true);
  assert.equal(
    isObsidianOpenLink("obsidian://open?path=%2Ftmp%2Fexample.md"),
    true,
  );
});

test("rejects non-open actions and ambiguous or extensible parameters", () => {
  for (const link of [
    "obsidian://new?vault=work&name=note",
    "obsidian://search?vault=work&query=secret",
    "obsidian://open?vault=work&file=note.md&x-success=https://example.com",
    "obsidian://open?vault=first&vault=second",
    "obsidian://open?vault=work&path=%2Ftmp%2Fnote.md",
  ]) {
    assert.equal(isObsidianOpenLink(link), false, link);
  }
});

test("rejects malformed or unsafe open links", () => {
  for (const link of [
    "obsidian://open",
    "obsidian://user@open?vault=work",
    "obsidian://open/extra?vault=work",
    "obsidian://open?vault=work#fragment",
    "obsidian://open?vault=%00work",
    "not a URL",
  ]) {
    assert.equal(isObsidianOpenLink(link), false, link);
  }
});
