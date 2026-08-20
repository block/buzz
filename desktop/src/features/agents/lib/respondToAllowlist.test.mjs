import assert from "node:assert/strict";
import test from "node:test";

import {
  allowlistChipPresentation,
  mergeAllowlist,
  parsePubkeyInput,
} from "./respondToAllowlist.ts";

const HEX_A = "a".repeat(64);
const HEX_B = "b".repeat(64);
const HEX_A_UPPER = "A".repeat(64);

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

test("parsePubkeyInput rejects npub-style strings (hex only)", () => {
  const npub = `npub1${"a".repeat(59)}`;
  const result = parsePubkeyInput(npub);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, [npub]);
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

test("allowlistChipPresentation prefers the display name", () => {
  assert.deepEqual(
    allowlistChipPresentation({
      displayName: "Ada Lovelace",
      nip05Handle: "ada@example.com",
      avatarUrl: "https://example.com/a.png",
    }),
    { name: "Ada Lovelace", avatarUrl: "https://example.com/a.png" },
  );
});

test("allowlistChipPresentation falls back to the NIP-05 handle", () => {
  assert.deepEqual(
    allowlistChipPresentation({
      displayName: "   ",
      nip05Handle: "ada@example.com",
      avatarUrl: null,
    }),
    { name: "ada@example.com", avatarUrl: null },
  );
});

test("allowlistChipPresentation reports no name when nothing resolved", () => {
  // The caller renders the truncated pubkey for a null name — an unresolved
  // profile must not turn into an empty-looking chip.
  assert.deepEqual(allowlistChipPresentation(undefined), {
    name: null,
    avatarUrl: null,
  });
  assert.deepEqual(
    allowlistChipPresentation({ displayName: null, nip05Handle: "  " }),
    { name: null, avatarUrl: null },
  );
});
