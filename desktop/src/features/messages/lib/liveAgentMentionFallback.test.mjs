import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveLivePersonaMentionPubkey,
  shouldSkipLocalStartForOnlineAgent,
} from "./liveAgentMentionFallback.ts";

const KIKU_LIVE = "a".repeat(64);
const KIKU_ORPHAN = "b".repeat(64);
const CRAWFORD = "c".repeat(64);

// ── resolveLivePersonaMentionPubkey ───────────────────────────────────

test("resolves a live agent whose name matches the persona mention", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [{ name: "Kiku", pubkey: KIKU_LIVE }],
    "Kiku",
    new Set(),
  );
  assert.equal(pubkey, KIKU_LIVE);
});

test("name matching is case- and whitespace-insensitive", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [{ name: "  kiku ", pubkey: KIKU_LIVE }],
    "KIKU",
    new Set(),
  );
  assert.equal(pubkey, KIKU_LIVE);
});

test("prefers the match that is a member of the current channel", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [
      { name: "Kiku", pubkey: KIKU_ORPHAN },
      { name: "Kiku", pubkey: KIKU_LIVE },
    ],
    "Kiku",
    new Set([KIKU_LIVE]),
  );
  assert.equal(pubkey, KIKU_LIVE);
});

test("falls back to the first match when no match is a channel member", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [
      { name: "Kiku", pubkey: KIKU_ORPHAN },
      { name: "Kiku", pubkey: KIKU_LIVE },
    ],
    "Kiku",
    new Set([CRAWFORD]),
  );
  assert.equal(pubkey, KIKU_ORPHAN);
});

test("returns normalized pubkeys", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [{ name: "Kiku", pubkey: ` ${KIKU_LIVE.toUpperCase()} ` }],
    "Kiku",
    new Set(),
  );
  assert.equal(pubkey, KIKU_LIVE);
});

test("returns null when no live agent matches the name", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [{ name: "Crawford", pubkey: CRAWFORD }],
    "Kiku",
    new Set(),
  );
  assert.equal(pubkey, null);
});

test("returns null for an empty or missing directory", () => {
  assert.equal(resolveLivePersonaMentionPubkey([], "Kiku", new Set()), null);
  assert.equal(
    resolveLivePersonaMentionPubkey(undefined, "Kiku", new Set()),
    null,
  );
});

test("returns null for a blank display name", () => {
  const pubkey = resolveLivePersonaMentionPubkey(
    [{ name: "Kiku", pubkey: KIKU_LIVE }],
    "   ",
    new Set(),
  );
  assert.equal(pubkey, null);
});

// ── shouldSkipLocalStartForOnlineAgent ────────────────────────────────

test("skips the local start when the agent is online on the relay", () => {
  assert.equal(shouldSkipLocalStartForOnlineAgent("online"), true);
});

test("does not skip for away, offline, or unknown presence", () => {
  assert.equal(shouldSkipLocalStartForOnlineAgent("away"), false);
  assert.equal(shouldSkipLocalStartForOnlineAgent("offline"), false);
  assert.equal(shouldSkipLocalStartForOnlineAgent(undefined), false);
});
