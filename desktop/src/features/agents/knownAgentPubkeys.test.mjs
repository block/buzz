import assert from "node:assert/strict";
import test from "node:test";

import { mergeKnownAgentPubkeys } from "./knownAgentPubkeys.ts";

const MANAGED =
  "1111111111111111111111111111111111111111111111111111111111111111";
const RELAY =
  "2222222222222222222222222222222222222222222222222222222222222222";
const FEED = "3333333333333333333333333333333333333333333333333333333333333333";

test("mergesAllThreeSources", () => {
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED }],
    [{ pubkey: RELAY }],
    [{ pubkey: FEED }],
  );

  assert.deepEqual([...merged].sort(), [MANAGED, RELAY, FEED].sort());
});

test("undefinedSources_yieldEmptySet", () => {
  const merged = mergeKnownAgentPubkeys(undefined, undefined, undefined);

  assert.equal(merged.size, 0);
});

test("normalisesCaseAndWhitespace_dedupingAcrossSources", () => {
  // The same agent appearing in multiple sources with different casing /
  // stray whitespace must collapse to one normalised entry, so membership
  // checks against normalizePubkey output always hit.
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED.toUpperCase() }],
    [{ pubkey: ` ${MANAGED}` }],
    [{ pubkey: MANAGED }],
  );

  assert.deepEqual([...merged], [MANAGED]);
});
