import assert from "node:assert/strict";
import { test } from "node:test";

import { templateMemberPubkeys } from "./templateMembers.ts";

const FIZZ = "b41ea9dccb4b6ad92951955cd373138a16184a3cba9184850053f581640e9ffc";

test("templateMemberPubkeys normalizes and dedups case-insensitively", () => {
  const out = templateMemberPubkeys([
    { pubkey: FIZZ.toUpperCase(), label: "Fizz" },
    { pubkey: ` ${FIZZ} \n`, label: "Fizz again" },
  ]);
  assert.deepEqual(out, [FIZZ]);
});

test("templateMemberPubkeys keeps first-seen order", () => {
  const a = "a".repeat(64);
  const b = "b".repeat(64);
  assert.deepEqual(
    templateMemberPubkeys([
      { pubkey: b, label: null },
      { pubkey: a, label: null },
    ]),
    [b, a],
  );
});

test("templateMemberPubkeys drops malformed entries", () => {
  for (const bad of [
    "",
    "abc",
    "z".repeat(64),
    "a".repeat(63),
    "a".repeat(65),
    `${"ab".repeat(32)}g`,
  ]) {
    const out = templateMemberPubkeys([
      { pubkey: bad, label: "x" },
      { pubkey: FIZZ, label: null },
    ]);
    assert.deepEqual(out, [FIZZ], `malformed ${bad} dropped, valid kept`);
  }
});

test("templateMemberPubkeys tolerates undefined and empty rosters", () => {
  assert.deepEqual(templateMemberPubkeys(undefined), []);
  assert.deepEqual(templateMemberPubkeys([]), []);
});
