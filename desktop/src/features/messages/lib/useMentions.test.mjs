import assert from "node:assert/strict";
import test from "node:test";

import {
  getMentionOffset,
  getMentionOffsets,
  hasMention,
} from "./hasMention.ts";
import { extractMentionPubkeys } from "./extractMentionPubkeys.ts";

// ── Plain @mention ────────────────────────────────────────────────────

test("matches @Name at start of string", () => {
  assert.equal(hasMention("@Alice hello", "Alice"), true);
});

test("matches @Name after whitespace", () => {
  assert.equal(hasMention("hey @Alice", "Alice"), true);
});

test("matches the first member in a parenthesized team expansion", () => {
  assert.equal(hasMention("Launch Team(@Planner @Builder)", "Planner"), true);
  assert.equal(hasMention("Launch Team(@Planner @Builder)", "Builder"), true);
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

test("returns every mention offset", () => {
  const text = "@Fast Fizz Codex and @Fast Fizz";
  assert.deepEqual(getMentionOffsets(text, "Fast Fizz"), [0, 21]);
});

test("selected longer mention excludes a prefix member at the same offset", () => {
  const pubkeys = extractMentionPubkeys({
    text: "Hive Codex(@Fast Fizz Codex)",
    selectedMentions: new Map([["Fast Fizz Codex", "codex-pubkey"]]),
    memberCandidates: [
      {
        kind: "identity",
        pubkey: "fast-fizz-pubkey",
        displayName: "Fast Fizz",
        isMember: true,
        isAgent: true,
      },
      {
        kind: "identity",
        pubkey: "codex-pubkey",
        displayName: "Fast Fizz Codex",
        isMember: true,
        isAgent: true,
      },
    ],
  });

  assert.deepEqual(pubkeys, ["codex-pubkey"]);
});

test("manual longer mention excludes its prefix member", () => {
  const pubkeys = extractMentionPubkeys({
    text: "@Fast Fizz Codex",
    selectedMentions: new Map(),
    memberCandidates: [
      {
        kind: "identity",
        pubkey: "fast-fizz-pubkey",
        displayName: "Fast Fizz",
        isMember: true,
        isAgent: true,
      },
      {
        kind: "identity",
        pubkey: "codex-pubkey",
        displayName: "Fast Fizz Codex",
        isMember: true,
        isAgent: true,
      },
    ],
  });

  assert.deepEqual(pubkeys, ["codex-pubkey"]);
});

test("manual prefix mentions choose the longest name at each offset", () => {
  const pubkeys = extractMentionPubkeys({
    text: "@Fast Fizz Codex, please pair with @Fast Fizz.",
    selectedMentions: new Map(),
    memberCandidates: [
      {
        kind: "identity",
        pubkey: "fast-fizz-pubkey",
        displayName: "Fast Fizz",
        isMember: true,
        isAgent: true,
      },
      {
        kind: "identity",
        pubkey: "codex-pubkey",
        displayName: "Fast Fizz Codex",
        isMember: true,
        isAgent: true,
      },
    ],
  });

  assert.deepEqual(pubkeys, ["fast-fizz-pubkey", "codex-pubkey"]);
});

test("keeps manually typed prefix member mentions at distinct offsets", () => {
  const pubkeys = extractMentionPubkeys({
    text: "@Fast Fizz Codex, please pair with @Fast Fizz.",
    selectedMentions: new Map([["Fast Fizz Codex", "codex-pubkey"]]),
    memberCandidates: [
      {
        kind: "identity",
        pubkey: "fast-fizz-pubkey",
        displayName: "Fast Fizz",
        isMember: true,
        isAgent: true,
      },
      {
        kind: "identity",
        pubkey: "codex-pubkey",
        displayName: "Fast Fizz Codex",
        isMember: true,
        isAgent: true,
      },
    ],
  });

  assert.deepEqual(pubkeys, ["codex-pubkey", "fast-fizz-pubkey"]);
});

// ── Markdown code ─────────────────────────────────────────────────────

test("ignores mentions in inline code", () => {
  assert.equal(hasMention("run `notify @Alice now`", "Alice"), false);
  assert.equal(hasMention("run ``notify `x` @Alice``", "Alice"), false);
});

test("ignores mentions in fenced code blocks", () => {
  assert.equal(
    hasMention("before\n```ts\nnotify(@Alice)\n```\nafter", "Alice"),
    false,
  );
  assert.equal(hasMention("~~~\r\n@Alice\r\n~~~", "Alice"), false);
});

test("ignores mentions in indented code blocks", () => {
  assert.equal(hasMention("before\n    @Alice\nafter", "Alice"), false);
  assert.equal(hasMention("before\n\t@Alice\nafter", "Alice"), false);
});

test("still matches prose mentions around code", () => {
  assert.equal(hasMention("`@Alice` then @Alice", "Alice"), true);
  assert.equal(hasMention("```\n@Alice\n```\n@Alice", "Alice"), true);
  assert.equal(hasMention("    @Alice\n@Alice", "Alice"), true);
});

test("preserves the original offset after masked code", () => {
  const text = "`@Alice` then @Alice";
  assert.equal(getMentionOffset(text, "Alice"), text.lastIndexOf("@Alice"));
});

test("does not treat escaped or unclosed backticks as code", () => {
  assert.equal(hasMention("\\` @Alice", "Alice"), true);
  assert.equal(hasMention("` @Alice", "Alice"), true);
});

test("requires matching inline-code delimiter lengths", () => {
  assert.equal(hasMention("`` @Alice ` still code ``", "Alice"), false);
  assert.equal(hasMention("`` @Alice `", "Alice"), true);
});
