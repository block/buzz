import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMentionPattern,
  buildPrefixPattern,
  PREFIX_LEAD_GROUP,
} from "./mentionPattern.ts";

/** All matches, with the leading boundary stripped the way the plugin does. */
function matches(pattern, text) {
  pattern.lastIndex = 0;
  const found = [];
  let match = pattern.exec(text);
  while (match) {
    const lead = match[PREFIX_LEAD_GROUP] ?? "";
    found.push({
      text: match[0].slice(lead.length),
      index: match.index + lead.length,
    });
    match = pattern.exec(text);
  }
  return found;
}

test("a mention is matched at the start of the text", () => {
  const pattern = buildMentionPattern(["alice"]);
  assert.deepEqual(matches(pattern, "@alice hi"), [
    { text: "@alice", index: 0 },
  ]);
});

test("a mention is matched after whitespace and after an opening paren", () => {
  const pattern = buildMentionPattern(["alice"]);
  assert.deepEqual(matches(pattern, "hi @alice"), [
    { text: "@alice", index: 3 },
  ]);
  assert.deepEqual(matches(pattern, "hi (@alice)"), [
    { text: "@alice", index: 4 },
  ]);
});

test("an email address is not a mention of its domain", () => {
  // "@alice" here is part of bob@alice.dev — the composer's own highlighter
  // draws the boundary in the same place, so the rendered message should too.
  const pattern = buildMentionPattern(["alice"]);
  assert.deepEqual(matches(pattern, "mail me at bob@alice.dev"), []);
});

test("the boundary does not eat the space between two adjacent mentions", () => {
  const pattern = buildMentionPattern(["alice", "bob"]);
  assert.deepEqual(matches(pattern, "@alice @bob"), [
    { text: "@alice", index: 0 },
    { text: "@bob", index: 7 },
  ]);
});

test("longest known name still wins over a shorter prefix of it", () => {
  const pattern = buildMentionPattern(["ali", "alice"]);
  assert.deepEqual(matches(pattern, "hi @alice"), [
    { text: "@alice", index: 3 },
  ]);
});

test("unknown names are not matched", () => {
  const pattern = buildMentionPattern(["alice"]);
  assert.deepEqual(matches(pattern, "hi @carol"), []);
});

test("with no known names the mention pattern never matches", () => {
  assert.deepEqual(matches(buildMentionPattern([]), "hi @alice"), []);
});

test("the generic channel fallback also requires a leading boundary", () => {
  const pattern = buildPrefixPattern("#", [], { fallbackToGeneric: true });
  assert.deepEqual(matches(pattern, "see #general"), [
    { text: "#general", index: 4 },
  ]);
  assert.deepEqual(matches(pattern, "issue-42#general"), []);
});

test("an opening paren opens a mention but not a channel", () => {
  // Exactly the split the composer's highlighter draws: `(?<=[\s(])@` versus
  // `(?<=\s)#`. Team expansions render as `Team (@ana @bo)`; channels have no
  // such form, so `(#general)` is not a channel link.
  assert.deepEqual(matches(buildMentionPattern(["alice"]), "(@alice)"), [
    { text: "@alice", index: 1 },
  ]);
  assert.deepEqual(
    matches(buildPrefixPattern("#", ["general"]), "(#general)"),
    [],
  );
  assert.deepEqual(
    matches(buildPrefixPattern("#", [], { fallbackToGeneric: true }), "(#gen)"),
    [],
  );
});
