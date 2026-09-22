import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { parsePubkeyInput } from "./nostrUtils.ts";

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

describe("parsePubkeyInput", () => {
  it("accepts a lowercase 64-char hex pubkey", () => {
    assert.equal(parsePubkeyInput(HEX), HEX);
  });

  it("lowercases an uppercase hex pubkey", () => {
    assert.equal(parsePubkeyInput(HEX.toUpperCase()), HEX);
  });

  it("decodes an npub to its hex pubkey", () => {
    assert.equal(parsePubkeyInput(NPUB), HEX);
  });

  it("normalizes a mixed-case npub to its canonical hex", () => {
    // Preexisting behavior: user input is lowercased before decoding, so a
    // mixed-case npub — invalid Bech32 as written — still resolves to the
    // identity. canonicalNpub is the strict counterpart (see ../lib/pubkey.ts).
    assert.equal(
      parsePubkeyInput(`${NPUB.slice(0, 10)}${NPUB.slice(10).toUpperCase()}`),
      HEX,
    );
  });

  it("tolerates surrounding whitespace from copy-paste", () => {
    assert.equal(parsePubkeyInput(`  ${NPUB}\n`), HEX);
    assert.equal(parsePubkeyInput(` ${HEX} `), HEX);
  });

  it("rejects an npub with a corrupted checksum", () => {
    assert.equal(parsePubkeyInput(`${NPUB.slice(0, -1)}q`), null);
  });

  it("rejects other bech32 entities such as nsec", () => {
    assert.equal(
      parsePubkeyInput(
        "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5",
      ),
      null,
    );
  });

  it("rejects hex of the wrong length", () => {
    assert.equal(parsePubkeyInput(HEX.slice(0, 63)), null);
    assert.equal(parsePubkeyInput(`${HEX}0`), null);
  });

  it("rejects degenerate npubs whose payload is not a 64-char identity", () => {
    // `npubEncode` happily encodes short payloads with valid checksums —
    // those are not identity keys and must never bind as one.
    assert.equal(parsePubkeyInput("npub1m6kmamcvty5gd"), null);
    assert.equal(parsePubkeyInput("npub106246s"), null);
  });

  it("rejects non-hex non-npub input", () => {
    assert.equal(parsePubkeyInput(""), null);
    assert.equal(parsePubkeyInput("alice"), null);
    assert.equal(parsePubkeyInput(`z${HEX.slice(1)}`), null);
  });
});
