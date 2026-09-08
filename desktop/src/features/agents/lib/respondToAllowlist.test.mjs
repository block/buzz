import assert from "node:assert/strict";
import test from "node:test";

import { mergeAllowlist, parsePubkeyInput } from "./respondToAllowlist.ts";

const HEX_A = "a".repeat(64);
const HEX_B = "b".repeat(64);
const HEX_A_UPPER = "A".repeat(64);
// Handoff vector, round-trip verified with nostr-tools nip19.
const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const HEX_NPUB =
  "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

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

test("parsePubkeyInput accepts npub entries and normalizes to hex", () => {
  const result = parsePubkeyInput(HEX_NPUB);
  assert.deepEqual(result.valid, [HEX]);
  assert.deepEqual(result.invalid, []);
});

test("parsePubkeyInput dedupes npub and hex spellings of one key", () => {
  const result = parsePubkeyInput(`${HEX_NPUB} ${HEX} ${HEX.toUpperCase()}`);
  assert.deepEqual(result.valid, [HEX]);
  assert.deepEqual(result.invalid, []);
});

test("parsePubkeyInput rejects corrupted-checksum npubs", () => {
  // Valid bech32 shape, corrupted checksum: must not bind as an identity.
  const corrupt = `${HEX_NPUB.slice(0, -2)}qq`;
  const result = parsePubkeyInput(corrupt);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, [corrupt]);
});

test("parsePubkeyInput rejects degenerate npubs and short hex", () => {
  // Checksum-valid bech32 whose payload is not a 64-char identity key —
  // npubEncode('deadbeef') — never binds as an identity.
  const result = parsePubkeyInput("npub1m6kmamcvty5gd deadbeef");
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, ["npub1m6kmamcvty5gd", "deadbeef"]);
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
  const merged = mergeAllowlist([HEX_A], [HEX_A_UPPER]);
  assert.deepEqual(merged, [HEX_A]);
});

test("mergeAllowlist skips invalid additions silently", () => {
  // Invalid additions are caller-validated; merge ignores them defensively.
  const merged = mergeAllowlist([HEX_A], ["not-hex", HEX_B]);
  assert.deepEqual(merged, [HEX_A, HEX_B]);
});

test("mergeAllowlist normalizes npub additions to canonical hex", () => {
  assert.deepEqual(mergeAllowlist([HEX_A], [HEX_NPUB]), [HEX_A, HEX]);
});

test("mergeAllowlist dedupes an npub addition already present as hex", () => {
  assert.deepEqual(mergeAllowlist([HEX], [HEX_NPUB]), [HEX]);
});
