import assert from "node:assert/strict";
import test from "node:test";

import { buildConfirmedImportBindings } from "./confirmImportBindings.ts";
import {
  KIND_IMPORT_IDENTITY_BINDING,
  KIND_IMPORT_IDENTITY_CLAIM,
} from "@/shared/constants/kinds";

const ALICE =
  "1111111111111111111111111111111111111111111111111111111111111111";
const MALLORY =
  "2222222222222222222222222222222222222222222222222222222222222222";

let clock = 1000;
function attestation(dTag, pubkey) {
  const createdAt = clock++;
  return {
    id: createdAt.toString(16).padStart(64, "0"),
    kind: KIND_IMPORT_IDENTITY_BINDING,
    pubkey: "admin".padEnd(64, "0"),
    created_at: createdAt,
    tags: [
      ["d", dTag],
      ["p", pubkey],
    ],
  };
}
function claim(dTag, author) {
  const createdAt = clock++;
  return {
    id: createdAt.toString(16).padStart(64, "0"),
    kind: KIND_IMPORT_IDENTITY_CLAIM,
    pubkey: author,
    created_at: createdAt,
    tags: [["d", dTag]],
  };
}

test("confirms only when attestation and claim agree", () => {
  const map = buildConfirmedImportBindings([
    attestation("slack:U1", ALICE),
    claim("slack:U1", ALICE),
  ]);
  assert.deepEqual([...map], [["slack:U1", ALICE]]);
});

test("attestation alone does not attribute (admin cannot forge authorship)", () => {
  const map = buildConfirmedImportBindings([attestation("slack:U1", ALICE)]);
  assert.equal(map.size, 0);
});

test("claim alone does not attribute (member cannot grab unvouched history)", () => {
  const map = buildConfirmedImportBindings([claim("slack:U1", MALLORY)]);
  assert.equal(map.size, 0);
});

test("a claim by a different pubkey than the attestation is rejected", () => {
  // Admin attests U1 -> ALICE, but MALLORY is the one who claimed U1.
  const map = buildConfirmedImportBindings([
    attestation("slack:U1", ALICE),
    claim("slack:U1", MALLORY),
  ]);
  assert.equal(map.size, 0);
});

test("newest attestation wins, and needs a claim matching the new pubkey", () => {
  // Admin re-attests U1 from ALICE to MALLORY; only ALICE had claimed it, so
  // the superseded pubkey's claim must not confirm the new attestation.
  const map = buildConfirmedImportBindings([
    attestation("slack:U1", ALICE),
    claim("slack:U1", ALICE),
    attestation("slack:U1", MALLORY),
  ]);
  assert.equal(map.size, 0);

  // Once MALLORY also claims, it confirms under the newest pubkey.
  const map2 = buildConfirmedImportBindings([
    attestation("slack:U1", ALICE),
    claim("slack:U1", ALICE),
    attestation("slack:U1", MALLORY),
    claim("slack:U1", MALLORY),
  ]);
  assert.deepEqual([...map2], [["slack:U1", MALLORY]]);
});

test("bound pubkey is lowercased on both sides before matching", () => {
  const upper = "ABCDEF".padEnd(64, "0"); // contains hex letters
  const lower = upper.toLowerCase();
  const map = buildConfirmedImportBindings([
    attestation("slack:U1", upper), // attestation p tag upper-cased
    claim("slack:U1", lower), // claim author lower-cased
  ]);
  // Case must not defeat the match, and the stored key is lowercase.
  assert.deepEqual([...map], [["slack:U1", lower]]);
});

test("same-second conflicting attestations resolve deterministically by id", () => {
  const first = attestation("slack:U1", ALICE);
  const second = attestation("slack:U1", MALLORY);
  first.created_at = 2000;
  second.created_at = 2000;
  first.id = "1".padStart(64, "0");
  second.id = "2".padStart(64, "0");
  const events = [
    claim("slack:U1", ALICE),
    claim("slack:U1", MALLORY),
    second,
    first,
  ];

  assert.deepEqual(
    [...buildConfirmedImportBindings(events)],
    [["slack:U1", MALLORY]],
  );
  assert.deepEqual(
    [...buildConfirmedImportBindings([...events].reverse())],
    [["slack:U1", MALLORY]],
  );
});
