import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { nsecEncode } from "nostr-tools/nip19";
import { generateSecretKey } from "nostr-tools/pure";

import { nsecToNpub } from "./nostrUtils.ts";

const VALID_NSEC = nsecEncode(generateSecretKey());

describe("nsecToNpub", () => {
  it("derives an npub from a lowercase nsec", () => {
    const npub = nsecToNpub(VALID_NSEC);
    assert.equal(typeof npub, "string");
    assert.equal(npub.startsWith("npub1"), true);
  });

  it("accepts an all-uppercase bech32 encoding", () => {
    assert.equal(nsecToNpub(VALID_NSEC.toUpperCase()), nsecToNpub(VALID_NSEC));
  });

  it("tolerates surrounding whitespace from copy-paste", () => {
    assert.equal(nsecToNpub(`  ${VALID_NSEC}\n`), nsecToNpub(VALID_NSEC));
    assert.equal(nsecToNpub(` ${VALID_NSEC} `), nsecToNpub(VALID_NSEC));
  });

  it("rejects an nsec with a corrupted checksum", () => {
    assert.equal(nsecToNpub(`${VALID_NSEC.slice(0, -1)}q`), null);
  });

  it("resolves mixed-case bech32 tolerantly", () => {
    // Bech32 spec requires uniform case; this parser is deliberately
    // permissive — a copy-paste that damaged only the casing should still
    // resolve since the checksum matches regardless.
    const mixed = `N${VALID_NSEC.slice(1)}`;
    assert.equal(nsecToNpub(mixed), nsecToNpub(VALID_NSEC));
  });

  it("rejects non-nsec inputs", () => {
    assert.equal(nsecToNpub("npub1whatever"), null);
    assert.equal(nsecToNpub(""), null);
    assert.equal(nsecToNpub("nsec1"), null);
  });
});
