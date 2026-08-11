import assert from "node:assert/strict";
import test from "node:test";
import { npubEncode } from "nostr-tools/nip19";

import { mergeAllowlist, parsePubkeyInput } from "./respondToAllowlist.ts";

const HEX_A =
  "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const HEX_B =
  "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
const HEX_A_UPPER = HEX_A.toUpperCase();
const NPUB_A = npubEncode(HEX_A);

test("parsePubkeyInput splits on commas, whitespace, and newlines", () => {
  const input = `${HEX_A}, ${HEX_B}\n${HEX_A_UPPER}`;
  const result = parsePubkeyInput(input);
  assert.deepEqual(result.valid, [HEX_A, HEX_B]);
  assert.deepEqual(result.invalid, []);
});

test("parsePubkeyInput lowercases and dedupes", () => {
  const result = parsePubkeyInput(`${HEX_A_UPPER} ${HEX_A}`);
  assert.deepEqual(result.valid, [HEX_A]);
});

test("parsePubkeyInput surfaces invalid entries separately", () => {
  const result = parsePubkeyInput(`notgood ${HEX_A} ${"z".repeat(64)}`);
  assert.deepEqual(result.valid, [HEX_A]);
  assert.deepEqual(result.invalid, ["notgood", "z".repeat(64)]);
});

test("parsePubkeyInput accepts canonical npubs and normalizes to protocol hex", () => {
  const result = parsePubkeyInput(`${NPUB_A} ${HEX_A}`);
  assert.deepEqual(result.valid, [HEX_A]);
  assert.deepEqual(result.invalid, []);
});

test("parsePubkeyInput rejects npubs with invalid checksums", () => {
  const invalidNpub = `${NPUB_A.slice(0, -1)}q`;
  const result = parsePubkeyInput(invalidNpub);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, [invalidNpub]);
});

test("parsePubkeyInput rejects wrong-length entries", () => {
  const shortHex = "a".repeat(63);
  const longHex = "a".repeat(65);
  const result = parsePubkeyInput(`${shortHex} ${longHex}`);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, [shortHex, longHex]);
});

test("parsePubkeyInput handles empty and whitespace-only input", () => {
  assert.deepEqual(parsePubkeyInput("").valid, []);
  assert.deepEqual(parsePubkeyInput("   \n\t  ").valid, []);
});

test("mergeAllowlist preserves existing order and appends new", () => {
  const merged = mergeAllowlist([HEX_A], [HEX_B]);
  assert.deepEqual(merged, [HEX_A, HEX_B]);
});

test("mergeAllowlist dedupes case-insensitively", () => {
  const merged = mergeAllowlist([NPUB_A], [HEX_A_UPPER]);
  assert.deepEqual(merged, [HEX_A]);
});

test("mergeAllowlist skips invalid additions silently", () => {
  // Invalid additions are caller-validated; merge ignores them defensively.
  const merged = mergeAllowlist([HEX_A], ["not-hex", HEX_B]);
  assert.deepEqual(merged, [HEX_A, HEX_B]);
});
