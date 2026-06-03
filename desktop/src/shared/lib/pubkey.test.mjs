import assert from "node:assert/strict";
import test from "node:test";

import { canonicalPubkeyOrThrow } from "./pubkey.ts";

const HEX = "35c18ae273fccfaf80d629e20e7f8721b90499379addff533054acc2504c12b4";
const NPUB = "npub1xhqc4cnnln86lqxk983qulu8yxusfxfhntwl75es2jkvy5zvz26qzr0685";

test("passes through canonical lowercase hex unchanged", () => {
  assert.equal(canonicalPubkeyOrThrow(HEX), HEX);
});

test("lowercases uppercase hex", () => {
  assert.equal(canonicalPubkeyOrThrow(HEX.toUpperCase()), HEX);
});

test("trims surrounding whitespace", () => {
  assert.equal(canonicalPubkeyOrThrow(`  ${HEX}\n`), HEX);
});

test("decodes npub to hex", () => {
  assert.equal(canonicalPubkeyOrThrow(NPUB), HEX);
});

test("trims then decodes npub", () => {
  assert.equal(canonicalPubkeyOrThrow(`  ${NPUB} `), HEX);
});

test("throws on empty string", () => {
  assert.throws(() => canonicalPubkeyOrThrow(""), /invalid pubkey/);
});

test("throws on too-short hex", () => {
  assert.throws(() => canonicalPubkeyOrThrow("35c18ae2"), /invalid pubkey/);
});

test("throws on non-hex garbage", () => {
  assert.throws(() => canonicalPubkeyOrThrow("not-a-pubkey"), /invalid pubkey/);
});

test("throws on malformed npub", () => {
  assert.throws(() => canonicalPubkeyOrThrow("npub1zzzz"));
});
