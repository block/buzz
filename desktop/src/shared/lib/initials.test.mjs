import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { getInitials } from "./initials.ts";

describe("getInitials", () => {
  it("filters punctuation before deriving initials", () => {
    assert.equal(getInitials("B (relay)"), "BR");
  });

  it("handles a leading symbol on a single word", () => {
    assert.equal(getInitials("(staging)"), "S");
  });

  it("still returns plain initials for normal names", () => {
    assert.equal(getInitials("Bravo Beta"), "BB");
  });

  it("returns empty for a symbol-only name", () => {
    assert.equal(getInitials("()"), "");
  });

  it("derives key-label initials from the compact key’s visible tail, not the npub prefix", () => {
    // Compact npub labels every start with npub1: the tail is the only
    // fragment that distinguishes one key-identified identity from another.
    assert.equal(getInitials("npub1z59…zwkg"), "ZW");
    assert.equal(getInitials("npub1m6k…zuz0"), "ZU");
    assert.equal(getInitials("npub1etl…wnmp"), "WN");
  });

  it("derives full-npub initials from the same tail fragment", () => {
    assert.equal(
      getInitials(
        "npub1z59jp0d24242424242424242424242424242424242424242zhwqnlzwkg",
      ),
      "ZW",
    );
  });

  it("leaves authored names that merely resemble npubs on the name path", () => {
    // Not a key-shaped label: wrong lengths, separators, alphabet, or casing
    // must keep the ordinary name derivation so an authored name is never
    // re-derived as a key just for resembling one.
    assert.equal(getInitials("Npub1 Person"), "NP");
    assert.equal(getInitials("npub1cool handle"), "NH");
    // Compact-label shape with a missing data or tail character.
    assert.equal(getInitials("npub1ab…wxy"), "NW");
    assert.equal(getInitials("npub1…zwkg"), "NZ");
    // Compact-label shape over letters outside the bech32 alphabet.
    assert.equal(getInitials("npub1bio…biob"), "NB");
    // Full-npub length with a non-bech32 character (uppercase tail).
    assert.equal(
      getInitials(
        "npub1z59jp0d24242424242424242424242424242424242424242zhwqnlzwkG",
      ),
      "N",
    );
  });

  it("requires a checksum-valid npub before deriving key-tail initials", () => {
    // Same length and alphabet as a real npub, but the checksum does not
    // decode: an authored lookalike must stay on the name path.
    assert.equal(
      getInitials(
        "npub1z59jp0d24242424242424242424242424242424242424242zhwqnlzwkq",
      ),
      "N",
    );
  });
});
