import assert from "node:assert/strict";
import test from "node:test";

import { collectMessageIdsForAuxBackfill } from "./auxBackfill.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";

function event(id, kind) {
  return {
    id,
    pubkey: "a".repeat(64),
    kind,
    created_at: 1_700_000_000,
    content: "",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
  };
}

function hex(char) {
  return char.repeat(64);
}

test("collects content-kind message ids (stream, v2, diff, system, jobs)", () => {
  const events = [
    event(hex("1"), 9), // stream message
    event(hex("2"), 40002), // v2 stream message
    event(hex("3"), 40008), // diff (own row)
    event(hex("4"), 40099), // system message
    event(hex("5"), 43001), // job request
  ];
  assert.deepEqual(collectMessageIdsForAuxBackfill(events), [
    hex("1"),
    hex("2"),
    hex("3"),
    hex("4"),
    hex("5"),
  ]);
});

test("excludes auxiliary kinds (reactions, edits, deletions)", () => {
  const events = [
    event(hex("1"), 9), // message — kept
    event(hex("2"), 7), // reaction — excluded
    event(hex("3"), 40003), // edit — excluded
    event(hex("4"), 5), // NIP-09 deletion — excluded
    event(hex("5"), 9005), // Buzz-native deletion — excluded
  ];
  assert.deepEqual(collectMessageIdsForAuxBackfill(events), [hex("1")]);
});

test("returns empty for a window of only auxiliary events", () => {
  const events = [event(hex("2"), 7), event(hex("3"), 40003)];
  assert.deepEqual(collectMessageIdsForAuxBackfill(events), []);
});
