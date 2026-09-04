import assert from "node:assert/strict";
import test from "node:test";
import {
  isTrustedMentionLabel,
  partitionMentionIdentitiesByLocalTrust,
} from "./mentionIdentityTrust.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);

for (const [label, aliases, pubkey, expected] of [
  ["Scout", ["Scout"], B, true],
  [`Scout (${B})`, ["Scout"], B, true],
  [` SCOUT\u00a0(${B.toUpperCase()}) `, ["scout"], B, true],
  [`Scout (${B}) 2`, ["Scout"], B, true],
  [`Scout (${B}) 12`, ["Scout"], B, true],
  [`Scout (${B})`, ["Scout"], A, false],
  [`Scout (${B})`, ["Other"], B, false],
  [`Scout (${B})`, [], B, false],
  [`Scout (${B})`, ["Scout"], undefined, false],
  [`Scout (${B}) 1`, ["Scout"], B, false],
  [`Scout (${B}) 02`, ["Scout"], B, false],
  [`Scout (${B.slice(0, 8)})`, ["Scout"], B, false],
  [`Scout (${B}) trailing`, ["Scout"], B, false],
]) {
  test(`clipboard label trust ${JSON.stringify({ label, aliases, pubkey })}`, () => {
    assert.equal(isTrustedMentionLabel(label, aliases, pubkey), expected);
  });
}

test("local and relay partitioning only vouches for a matching key and trusted alias", () => {
  const valid = { label: `Scout (${B})`, pubkey: B };
  const mismatched = { label: `Scout (${B})`, pubkey: A };
  const forged = { label: `Other (${B})`, pubkey: B };
  const records = [valid, mismatched, forged];
  const local = partitionMentionIdentitiesByLocalTrust(records, () => []);
  assert.deepEqual(local, { trusted: [], unresolved: records });
  assert.deepEqual(
    partitionMentionIdentitiesByLocalTrust(local.unresolved, () => ["Scout"]),
    { trusted: [valid], unresolved: [mismatched, forged] },
  );
});
