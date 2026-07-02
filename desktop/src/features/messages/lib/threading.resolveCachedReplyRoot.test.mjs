import assert from "node:assert/strict";
import test from "node:test";

import { resolveCachedReplyRoot, resolveReplyRootId } from "./threading.ts";

// resolveCachedReplyRoot must mirror resolve_thread_ref in
// desktop/src-tauri/src/commands/messages.rs EXACTLY: a non-null result is
// what the Rust side would have fetched from the relay, so any divergence
// here silently changes reply threading. The relay's ingest-side ancestry
// check turns a wrong root into a rejected send — these tests keep us off
// that path entirely.

const ROOT = "a".repeat(64);
const PARENT = "b".repeat(64);
const OTHER = "c".repeat(64);

const ev = (id, kind, tags = []) => ({ id, kind, tags });

test("cache miss returns null (falls back to relay)", () => {
  assert.equal(resolveCachedReplyRoot(PARENT, []), null);
  assert.equal(resolveCachedReplyRoot(PARENT, [ev(OTHER, 9)]), null);
});

test("parent kind outside the Rust resolver's allowlist returns null", () => {
  // resolve_thread_ref queries kinds [9, 40002, 45001, 45003, 48100] only —
  // for any other cached kind the relay path would report "parent event not
  // found", so the cached path must decline rather than diverge.
  for (const kind of [1, 7, 45021, 40001]) {
    assert.equal(
      resolveCachedReplyRoot(PARENT, [ev(PARENT, kind)]),
      null,
      `kind ${kind} must fall back`,
    );
  }
  for (const kind of [9, 40002, 45001, 45003, 48100]) {
    assert.equal(
      resolveCachedReplyRoot(PARENT, [ev(PARENT, kind)]),
      PARENT,
      `kind ${kind} must resolve`,
    );
  }
});

test("tagless parent is its own root", () => {
  assert.equal(resolveCachedReplyRoot(PARENT, [ev(PARENT, 9)]), PARENT);
});

test("root marker wins over reply marker", () => {
  const parent = ev(PARENT, 9, [
    ["e", OTHER, "", "reply"],
    ["e", ROOT, "", "root"],
  ]);
  assert.equal(resolveCachedReplyRoot(PARENT, [parent]), ROOT);
});

test("reply marker used when no root marker (parent was a direct reply)", () => {
  const parent = ev(PARENT, 9, [["e", ROOT, "", "reply"]]);
  assert.equal(resolveCachedReplyRoot(PARENT, [parent]), ROOT);
});

test("last marker of each kind wins, matching the Rust tag walk", () => {
  const parent = ev(PARENT, 9, [
    ["e", OTHER, "", "root"],
    ["e", ROOT, "", "root"],
  ]);
  assert.equal(resolveCachedReplyRoot(PARENT, [parent]), ROOT);
});

test("marker pointing at the parent itself collapses to the parent", () => {
  // Rust: `Some(hex) if hex != parent_event_id` — a self-referential tag
  // means the parent IS the root.
  const parent = ev(PARENT, 9, [["e", PARENT, "", "reply"]]);
  assert.equal(resolveCachedReplyRoot(PARENT, [parent]), PARENT);
});

test("short/unmarked e-tags are ignored, as in the Rust s.len() >= 4 guard", () => {
  const parent = ev(PARENT, 9, [
    ["e", OTHER], // no marker — mention-style tag
    ["e", ROOT, ""], // len 3 — no marker slot
  ]);
  assert.equal(resolveCachedReplyRoot(PARENT, [parent]), PARENT);
});

test("does NOT inherit resolveReplyRootId's parent-id fallback on cache miss", () => {
  // resolveReplyRootId returns the parent id when the parent isn't cached —
  // safe for optimistic UI, catastrophic here: it would label a nested reply
  // as a thread root. The cached resolver must return null instead.
  assert.equal(resolveReplyRootId(PARENT, []), PARENT);
  assert.equal(resolveCachedReplyRoot(PARENT, []), null);
});
