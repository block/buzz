import assert from "node:assert/strict";
import test from "node:test";
import { formatMentionDisplayLabel } from "./mentionDisplay.ts";
import { truncatePubkey } from "./pubkey.ts";

const KEY = `150b20bd${"a".repeat(52)}15dc`;

test("compact mention display uses the member-list formatter and keeps collision suffixes", () => {
  for (const suffix of ["", " 2", " 10"]) {
    assert.equal(
      formatMentionDisplayLabel(`Bad Janet (${KEY})${suffix}`, KEY),
      `Bad Janet (${truncatePubkey(KEY)})${suffix}`,
    );
  }
  assert.equal(formatMentionDisplayLabel(KEY, KEY), truncatePubkey(KEY));
  assert.equal(
    formatMentionDisplayLabel(KEY.toUpperCase(), KEY),
    truncatePubkey(KEY.toUpperCase()),
  );
});

test("display leaves unbound, mismatched, malformed and ordinary labels literal", () => {
  for (const [label, key] of [
    [`Bad Janet (${KEY})`, undefined],
    [`Bad Janet (${KEY})`, "b".repeat(64)],
    [`Bad Janet (${KEY})`, "bad-key"],
    [`Bad Janet (${KEY}) 1`, KEY],
    [`Bad Janet (${KEY}) notes`, KEY],
    [`Release ${KEY}`, KEY],
    ["Bad Janet", KEY],
  ])
    assert.equal(formatMentionDisplayLabel(label, key), label);
});

test("matching compact keys do not become identity keys", () => {
  const other = KEY.replace("aaaa", "bbbb");
  assert.notEqual(KEY, other);
  assert.equal(
    formatMentionDisplayLabel(`Scout (${KEY})`, KEY),
    formatMentionDisplayLabel(`Scout (${other})`, other),
  );
  assert.equal(
    formatMentionDisplayLabel(`Scout (${KEY})`, other),
    `Scout (${KEY})`,
  );
});
