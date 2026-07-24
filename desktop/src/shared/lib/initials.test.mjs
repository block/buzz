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

  it("derives whole-character initials from astral-plane letters", () => {
    const initials = getInitials("𠮷野 太郎");
    assert.equal(initials, "𠮷太");
    for (const ch of initials) {
      const cp = ch.codePointAt(0);
      assert.ok(
        cp < 0xd800 || cp > 0xdfff,
        `unexpected lone surrogate U+${cp.toString(16)}`,
      );
    }
  });

  it("derives an initial from an astral-plane single word", () => {
    assert.equal(getInitials("𝓐lice"), "𝓐");
  });
});
