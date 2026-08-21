import assert from "node:assert/strict";
import test from "node:test";

import { detectPrefixQuery } from "./detectPrefixQuery.ts";

const CHANNELS = ["buzz-bugs", "buzz dev", "general"];
const PEOPLE = ["alice", "bob jones"];

// Helper: detect at end-of-string (the usual cursor position while typing).
const at = (prefix, text, names) =>
  detectPrefixQuery(prefix, text, text.length, names);

// ── Existing behavior: start-of-string and after whitespace still trigger ─────

test("triggers at start of string", () => {
  assert.deepEqual(at("#", "#buzz", CHANNELS), {
    query: "buzz",
    startIndex: 0,
  });
});

test("triggers after whitespace", () => {
  assert.deepEqual(at("#", "go to #buzz", CHANNELS), {
    query: "buzz",
    startIndex: 6,
  });
});

test("does NOT trigger when glued to a word character", () => {
  assert.equal(at("#", "foo#buzz", CHANNELS), null);
  assert.equal(at("@", "email@bob", PEOPLE), null);
});

// ── The bug fix: opening brackets count as a boundary ─────────────────────────

test("7796a4f4: ( before # triggers channel query", () => {
  assert.deepEqual(at("#", "(#buzz", CHANNELS), {
    query: "buzz",
    startIndex: 1,
  });
});

test("7796a4f4: [ and { before # also trigger", () => {
  assert.deepEqual(at("#", "[#buzz", CHANNELS), {
    query: "buzz",
    startIndex: 1,
  });
  assert.deepEqual(at("#", "{#buzz", CHANNELS), {
    query: "buzz",
    startIndex: 1,
  });
});

test("7796a4f4: ( before @ triggers mention query", () => {
  assert.deepEqual(at("@", "(@alice", PEOPLE), {
    query: "alice",
    startIndex: 1,
  });
});

test("bracket mid-sentence (word before the bracket) still triggers", () => {
  // The char immediately before the prefix is `(`, which is the boundary —
  // what precedes the bracket is irrelevant.
  assert.deepEqual(at("#", "see also(#general", CHANNELS), {
    query: "general",
    startIndex: 9,
  });
});

test("nested brackets: (( before # triggers, startIndex at the prefix", () => {
  assert.deepEqual(at("#", "((#buzz", CHANNELS), {
    query: "buzz",
    startIndex: 2,
  });
});

// ── Multi-word path: bracket boundary works for space-containing names ─────────

test("multi-word channel name after ( resolves via multi-word path", () => {
  assert.deepEqual(at("#", "(#buzz de", CHANNELS), {
    query: "buzz de",
    startIndex: 1,
  });
});

test("multi-word person name after ( resolves via multi-word path", () => {
  assert.deepEqual(at("@", "ping (@bob jo", PEOPLE), {
    query: "bob jo",
    startIndex: 6,
  });
});

test("multi-word: glued-to-word prefix still rejected", () => {
  // `x#buzz de` — `#` preceded by `x`, no boundary → no query even though
  // "buzz de" is a prefix of a known channel.
  assert.equal(at("#", "x#buzz de", CHANNELS), null);
});

// ── Completed mention: exact name + trailing space closes the query ───────────

test("exact name followed by space does not stay open for a longer name", () => {
  // "pinky" is complete; "pinky and the brain" sharing the prefix must not
  // keep the popup open and steal Enter/Tab.
  const names = ["pinky", "brain", "pinky and the brain"];
  assert.equal(at("@", "@pinky ", names), null);
});

test("typing past the space toward the longer name re-opens the query", () => {
  const names = ["pinky", "brain", "pinky and the brain"];
  assert.deepEqual(at("@", "@pinky a", names), {
    query: "pinky a",
    startIndex: 0,
  });
});

test("multi-word name still completes word by word when no shorter exact match", () => {
  assert.deepEqual(at("@", "@bob ", PEOPLE), {
    query: "bob ",
    startIndex: 0,
  });
});

// ── Empty / no-match guards unchanged ─────────────────────────────────────────

test("bare prefix after ( yields empty single-word query, not multi-word", () => {
  // Fast path matches with empty query (dropdown shows all) — consistent with
  // bare `#` at start.
  assert.deepEqual(at("#", "(#", CHANNELS), { query: "", startIndex: 1 });
});

test("no prefix present → null", () => {
  assert.equal(at("#", "just text", CHANNELS), null);
});
