import assert from "node:assert/strict";
import test from "node:test";

import {
  isChannelUuid,
  isHexPubkey,
  normalizeFromHandle,
  normalizeInChannel,
  parseSearchOperators,
} from "./parseSearchOperators.ts";

test("leaves plain text unchanged", () => {
  assert.deepEqual(parseSearchOperators("  deploy  "), {
    text: "deploy",
    from: null,
    in: null,
    since: null,
    until: null,
  });
});

test("extracts from / in / after / before and keeps remaining FTS text", () => {
  const parsed = parseSearchOperators(
    "deploy from:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef in:#general after:2024-01-15 before:2024-02-01 status",
  );
  assert.equal(parsed.text, "deploy status");
  assert.equal(
    parsed.from,
    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
  );
  assert.equal(parsed.in, "#general");
  assert.equal(parsed.since, Math.floor(new Date(2024, 0, 15).getTime() / 1000));
  assert.equal(parsed.until, Math.floor(new Date(2024, 1, 1).getTime() / 1000));
});

test("keeps invalid date operators in the FTS text", () => {
  const parsed = parseSearchOperators("notes after:yesterday before:soon");
  assert.equal(parsed.text, "notes after:yesterday before:soon");
  assert.equal(parsed.since, null);
  assert.equal(parsed.until, null);
});

test("later operators of the same kind win", () => {
  const parsed = parseSearchOperators(
    "from:@alice from:@bob in:one in:two after:2024-01-01 after:2024-03-01",
  );
  assert.equal(parsed.from, "@bob");
  assert.equal(parsed.in, "two");
  assert.equal(parsed.since, Math.floor(new Date(2024, 2, 1).getTime() / 1000));
  assert.equal(parsed.text, "");
});

test("rejects impossible calendar dates", () => {
  const parsed = parseSearchOperators("x after:2024-02-30");
  assert.equal(parsed.since, null);
  assert.equal(parsed.text, "x after:2024-02-30");
});

test("helpers recognize hex pubkeys and channel uuids", () => {
  assert.equal(
    isHexPubkey("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
    true,
  );
  assert.equal(isHexPubkey("npub1abc"), false);
  assert.equal(
    isChannelUuid("11111111-1111-1111-1111-111111111111"),
    true,
  );
  assert.equal(normalizeFromHandle("@alice"), "alice");
  assert.equal(normalizeInChannel("#general"), "general");
});
