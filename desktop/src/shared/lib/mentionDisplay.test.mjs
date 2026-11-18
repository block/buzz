import assert from "node:assert/strict";
import test from "node:test";
import {
  formatLegacyMentionDisplayLabel,
  formatMentionDisplayLabel,
} from "./mentionDisplay.ts";
import { truncateNpub, truncatePubkey } from "./pubkey.ts";

const KEY = `150b20bd${"a".repeat(52)}15dc`;
// npubEncode(KEY), pinned so a formatter regression cannot pass by
// re-deriving the expectation from the code under test.
const KEY_NPUB_COMPACT = "npub1z59…zwkg";

test("compact mention display uses the member-list formatter and keeps collision suffixes", () => {
  for (const suffix of ["", " 2", " 10"]) {
    assert.equal(
      formatMentionDisplayLabel(`Bad Janet (${KEY})${suffix}`, KEY),
      `Bad Janet (${truncateNpub(KEY)})${suffix}`,
    );
  }
  assert.equal(formatMentionDisplayLabel(KEY, KEY), KEY_NPUB_COMPACT);
  assert.equal(
    formatMentionDisplayLabel(KEY.toUpperCase(), KEY),
    KEY_NPUB_COMPACT,
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

test("legacy display keeps the retired hex compaction byte-exact for clipboard validation", () => {
  for (const suffix of ["", " 2", " 10"]) {
    assert.equal(
      formatLegacyMentionDisplayLabel(`Bad Janet (${KEY})${suffix}`, KEY),
      `Bad Janet (${truncatePubkey(KEY)})${suffix}`,
    );
  }
  assert.equal(formatLegacyMentionDisplayLabel(KEY, KEY), truncatePubkey(KEY));
});

test("matching compact keys do not become identity keys", () => {
  const other = KEY.replace("aaaa", "bbbb");
  assert.notEqual(KEY, other);
  // The two keys collide under the old hex first8…last4 truncation yet
  // compact to distinguishable npubs: npub display keeps them apart.
  assert.equal(truncatePubkey(KEY), truncatePubkey(other));
  assert.notEqual(
    formatMentionDisplayLabel(`Scout (${KEY})`, KEY),
    formatMentionDisplayLabel(`Scout (${other})`, other),
  );
  assert.equal(
    formatMentionDisplayLabel(`Scout (${KEY})`, other),
    `Scout (${KEY})`,
  );
});
