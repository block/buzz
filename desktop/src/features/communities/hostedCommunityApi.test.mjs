/**
 * Unit tests for the hosted-identity usable-bound-key predicate.
 *
 * Hosted surfaces gate connected/readiness/create/connect decisions on
 * whether the identity payload carries an authoritative `pubkey_hex` that
 * normalizes to a hex key — never on the mere presence of the identity
 * object, and never on the display parser's npub-accepting domain. These
 * tests pin the predicate's boundary semantics; the E2E specs assert the
 * rendered fail-closed states through the real surfaces.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { npubEncode } from "nostr-tools/nip19";

import {
  normalizedBoundKeyHex,
  usableBoundIdentityNpub,
} from "./hostedCommunityApi.ts";

const BOUND_HEX = "deadbeef".repeat(8);

test("returns the canonical npub for a valid bound hex", () => {
  assert.equal(
    usableBoundIdentityNpub({ pubkey_hex: BOUND_HEX }),
    npubEncode(BOUND_HEX),
  );
});

test("normalizes mixed-case bound hex to the canonical npub", () => {
  assert.equal(
    usableBoundIdentityNpub({ pubkey_hex: BOUND_HEX.toUpperCase() }),
    npubEncode(BOUND_HEX),
  );
});

test("npub stored in pubkey_hex is never a usable hex binding", () => {
  // A checksum-valid npub — even one spelling the very key this device
  // signs with — is the display field's domain. Stored in the authoritative
  // hex field it is not a hex key, so it must fail closed: never usable,
  // never a comparison operand, never a pending-local bypass that reads the
  // account as connected because the raw comparison was skipped.
  assert.equal(normalizedBoundKeyHex(npubEncode(BOUND_HEX)), null);
  assert.equal(
    usableBoundIdentityNpub({ pubkey_hex: npubEncode(BOUND_HEX) }),
    null,
  );
  // An all-uppercase NPUB spelling is equally rejected.
  assert.equal(
    normalizedBoundKeyHex(npubEncode(BOUND_HEX).toUpperCase()),
    null,
  );
});

test("same-key normalization: padded hex is the key it spells, on both sides", () => {
  // Whitespace-padded, mixed-case hex normalizes to the identical value
  // usability and comparisons use, so the same key never demands a
  // delete/rebind of an identity the device already holds.
  const padded = `  ${BOUND_HEX.toUpperCase()}  `;
  assert.equal(normalizedBoundKeyHex(padded), BOUND_HEX);
  assert.equal(normalizedBoundKeyHex(BOUND_HEX), BOUND_HEX);
  assert.equal(
    usableBoundIdentityNpub({ pubkey_hex: padded }),
    npubEncode(BOUND_HEX),
  );
});

test("returns null when the identity object is absent", () => {
  assert.equal(usableBoundIdentityNpub(null), null);
  assert.equal(usableBoundIdentityNpub(undefined), null);
});

test("returns null when the authoritative key field is missing or empty", () => {
  // npub-only payload: the server-sent `npub` is an independent, unverified
  // spelling and must never substitute for the authoritative key.
  assert.equal(usableBoundIdentityNpub({ npub: npubEncode(BOUND_HEX) }), null);
  assert.equal(usableBoundIdentityNpub({}), null);
  assert.equal(usableBoundIdentityNpub({ pubkey_hex: "" }), null);
});

test("returns null when the authoritative key cannot encode a key", () => {
  // Non-hex alphabet.
  assert.equal(usableBoundIdentityNpub({ pubkey_hex: "zz".repeat(32) }), null);
  // Valid hex alphabet, wrong length for an identity key.
  assert.equal(usableBoundIdentityNpub({ pubkey_hex: "f".repeat(63) }), null);
  assert.equal(usableBoundIdentityNpub({ pubkey_hex: "f".repeat(65) }), null);
  // Whitespace is not an identity key.
  assert.equal(usableBoundIdentityNpub({ pubkey_hex: "  " }), null);
});

test("the string normalizer rejects the same non-key values directly", () => {
  assert.equal(normalizedBoundKeyHex(null), null);
  assert.equal(normalizedBoundKeyHex(undefined), null);
  assert.equal(normalizedBoundKeyHex("zz".repeat(32)), null);
  assert.equal(normalizedBoundKeyHex("f".repeat(63)), null);
  assert.equal(normalizedBoundKeyHex("f".repeat(65)), null);
  assert.equal(normalizedBoundKeyHex("  "), null);
});
