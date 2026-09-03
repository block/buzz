/**
 * Pure-logic tests for private-key normalization (nsec vs buzz-admin hex).
 */
import assert from "node:assert/strict";
import { describe, test } from "node:test";

import { hexToBytes } from "@noble/hashes/utils.js";
import { nsecEncode } from "nostr-tools/nip19";
import { generateSecretKey, getPublicKey } from "nostr-tools/pure";

import {
  normalizePrivateKeyToNsec,
  nsecToNpub,
  pubkeyToNpub,
} from "./nostrUtils.ts";

const SECRET_HEX =
  "0000000000000000000000000000000000000000000000000000000000000001";
const SECRET_NSEC = nsecEncode(hexToBytes(SECRET_HEX));
const EXPECTED_NPUB = pubkeyToNpub(getPublicKey(hexToBytes(SECRET_HEX)));

describe("normalizePrivateKeyToNsec", () => {
  test("accepts nsec1 bech32", () => {
    assert.equal(normalizePrivateKeyToNsec(`  ${SECRET_NSEC}\n`), SECRET_NSEC);
  });

  test("accepts 64-char hex (buzz-admin generate-key output)", () => {
    assert.equal(normalizePrivateKeyToNsec(SECRET_HEX), SECRET_NSEC);
    assert.equal(
      normalizePrivateKeyToNsec(SECRET_HEX.toUpperCase()),
      SECRET_NSEC,
    );
  });

  test("rejects garbage", () => {
    assert.equal(normalizePrivateKeyToNsec("nsec1notvalid"), null);
    assert.equal(normalizePrivateKeyToNsec("00"), null);
    assert.equal(normalizePrivateKeyToNsec("npub1whatever"), null);
  });
});

describe("nsecToNpub", () => {
  test("derives npub from nsec and hex secrets", () => {
    assert.equal(nsecToNpub(SECRET_NSEC), EXPECTED_NPUB);
    assert.equal(nsecToNpub(SECRET_HEX), EXPECTED_NPUB);
    const random = generateSecretKey();
    const hex = Buffer.from(random).toString("hex");
    assert.equal(nsecToNpub(hex), nsecToNpub(nsecEncode(random)));
  });

  test("returns null for incomplete input", () => {
    assert.equal(nsecToNpub("nsec1"), null);
    assert.equal(nsecToNpub("00"), null);
  });
});
