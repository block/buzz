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
});

describe("getInitials beyond the BMP", () => {
  it("keeps a whole astral letter instead of half a surrogate pair", () => {
    // U+20000, CJK Extension B — an ordinary character in some names.
    const initials = getInitials("\u{20000}明");
    assert.equal(initials, "\u{20000}");
    assert.equal([...initials].length, 1);
  });

  it("keeps both initials whole when both are astral", () => {
    const initials = getInitials("\u{20000}\u{20001} \u{20002}\u{20003}");
    assert.equal(initials, "\u{20000}\u{20002}");
    assert.equal([...initials].length, 2);
  });

  it("mixes an astral first name with an ordinary surname", () => {
    assert.equal(getInitials("\u{1D400}da Lovelace"), "\u{1D400}L");
  });
});

describe("getInitials with combining marks", () => {
  it("does not split a word at a vowel sign", () => {
    // अनिल कुमार — the vowel sign in अनिल used to split the word, so the
    // second initial came from the middle of the first name. कु is one
    // cluster: the surname's vowel sign belongs to its consonant.
    assert.equal(getInitials("अनिल कुमार"), "अकु");
  });

  it("gives a one-word name one initial", () => {
    assert.equal(getInitials("नमस्ते"), "न");
  });

  it("handles a Burmese name the same way", () => {
    assert.equal(getInitials("မောင်မောင်"), "မေ");
  });

  it("still strips punctuation that is not a mark", () => {
    assert.equal(getInitials("B (relay)"), "BR");
  });
});

describe("getInitials takes a grapheme cluster, not a code point", () => {
  it("keeps a decomposed accent with its letter", () => {
    // NFD: E + U+0301. A code point initial dropped the accent entirely.
    // The result stays decomposed — the initial is the input's own cluster,
    // not a renormalized one — so compare against the decomposed form.
    assert.equal(getInitials("E\u0301lodie Durand"), "E\u0301D");
    assert.equal(getInitials("E\u0301lodie Durand").normalize("NFC"), "ÉD");
  });

  it("does not split a cluster joined by a zero-width joiner", () => {
    // क्‍ष is one cluster; ZWJ is neither a letter nor a mark, so it used to
    // act as a word separator and produce two initials from one word.
    assert.equal(getInitials("\u0915\u094D\u200D\u0937 Name"), "क्‍षN");
  });

  it("still returns nothing for a name with no letters", () => {
    assert.equal(getInitials("()"), "");
  });
});
