import assert from "node:assert/strict";
import test from "node:test";

import { findPubkeyTruncations } from "./check-pubkey-truncation-core.mjs";

test("flags a truncation written on one line", () => {
  const found = findPubkeyTruncations("const short = pubkey.slice(0, 8);\n");
  assert.deepEqual(
    found.map((entry) => entry.line),
    [1],
  );
});

test("flags a truncation the formatter wrapped onto the next line", () => {
  const found = findPubkeyTruncations(
    ["const short =", "  authorPubkey", "    .slice(0, 8);", ""].join("\n"),
  );
  assert.deepEqual(
    found.map((entry) => entry.line),
    [2],
  );
});

test("flags substring and substr as well as slice", () => {
  const found = findPubkeyTruncations(
    ["a = pubkey.substring(0, 8);", "b = pubkey.substr(0, 8);", ""].join("\n"),
  );
  assert.deepEqual(
    found.map((entry) => entry.line),
    [1, 2],
  );
});

test("flags optional-chained and suffixed pubkey identifiers", () => {
  const found = findPubkeyTruncations(
    ["a = npub?.slice(0, 8);", "b = recipientPubkeyHex.slice(0, 8);", ""].join(
      "\n",
    ),
  );
  assert.deepEqual(
    found.map((entry) => entry.line),
    [1, 2],
  );
});

test("reports the line the identifier starts on, not the one holding the call", () => {
  const [found] = findPubkeyTruncations(
    ["// leading comment", "const short = authorPubkey", "  .slice(0, 8);", ""].join(
      "\n",
    ),
  );
  assert.equal(found.line, 2);
  assert.equal(found.text, "const short = authorPubkey");
});

test("leaves unrelated identifiers alone", () => {
  const found = findPubkeyTruncations(
    ["a = messageId.slice(0, 8);", "b = name.substring(0, 4);", ""].join("\n"),
  );
  assert.deepEqual(found, []);
});

test("finds every occurrence in a file, not just the first", () => {
  const found = findPubkeyTruncations(
    ["a = pubkey.slice(0, 8);", "b = npub.slice(0, 8);", ""].join("\n"),
  );
  assert.equal(found.length, 2);
});
