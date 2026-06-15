import assert from "node:assert/strict";
import test from "node:test";

import { hasMention } from "./hasMention.ts";

// ── Plain @mention ────────────────────────────────────────────────────

test("matches @Name at start of string", () => {
  assert.equal(hasMention("@Alice hello", "Alice"), true);
});

test("matches @Name after whitespace", () => {
  assert.equal(hasMention("hey @Alice", "Alice"), true);
});

test("matches @Name at end of string", () => {
  assert.equal(hasMention("hello @Alice", "Alice"), true);
});

test("match is case-insensitive", () => {
  assert.equal(hasMention("@alice", "Alice"), true);
  assert.equal(hasMention("@ALICE", "Alice"), true);
});

test("does not match without @ prefix", () => {
  assert.equal(hasMention("Alice hello", "Alice"), false);
});

test("does not match @Name embedded in a word (email-style)", () => {
  assert.equal(hasMention("user@Alice.com", "Alice"), false);
});

// ── Bold-wrapped mentions (**@Name**) ─────────────────────────────────

test("matches **@Name** (bold-wrapped)", () => {
  assert.equal(hasMention("**@Alice**", "Alice"), true);
});

test("matches **@Name** after whitespace", () => {
  assert.equal(hasMention("hey **@Alice**", "Alice"), true);
});

test("matches *@Name* (italic-wrapped)", () => {
  assert.equal(hasMention("*@Alice*", "Alice"), true);
});

test("matches ***@Name*** (bold+italic-wrapped)", () => {
  assert.equal(hasMention("***@Alice***", "Alice"), true);
});

test("matches __@Name__ (underscore bold-wrapped)", () => {
  assert.equal(hasMention("__@Alice__", "Alice"), true);
});

test("matches _@Name_ (underscore italic-wrapped)", () => {
  assert.equal(hasMention("_@Alice_", "Alice"), true);
});

test("matches ||@Name|| (spoiler-wrapped)", () => {
  assert.equal(hasMention("||@Alice||", "Alice"), true);
});

test("matches @Name at the end of spoiler content", () => {
  assert.equal(hasMention("||hi @Alice||", "Alice"), true);
});

// ── Boundary conditions ───────────────────────────────────────────────

test("matches @Name followed by punctuation", () => {
  assert.equal(hasMention("@Alice, hello", "Alice"), true);
  assert.equal(hasMention("@Alice!", "Alice"), true);
  assert.equal(hasMention("@Alice.", "Alice"), true);
  assert.equal(hasMention("@Alice?", "Alice"), true);
});

test("matches multi-word display name", () => {
  assert.equal(hasMention("@John Doe said hi", "John Doe"), true);
});

test("matches multi-word display name bold-wrapped", () => {
  assert.equal(hasMention("**@John Doe**", "John Doe"), true);
});

test("handles regex special characters in name", () => {
  assert.equal(hasMention("@alice (admin)", "alice (admin)"), true);
});

test("does not false-positive on partial name match", () => {
  // "Al" should not match inside "@Alice"
  assert.equal(hasMention("@Alice", "Al"), false);
});
