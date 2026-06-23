import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAuxDeletionFilter,
  buildChannelReactionAuxFilter,
  buildChannelStructuralAuxFilter,
} from "./relayChannelFilters.ts";

const CHANNEL = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const IDS = [
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
];

// Regression: reaction (kind:7) and reaction-removal (kind:5) events carry only
// an `e` tag, no channel `h` tag. An `#h`-scoped aux query never matches them,
// so removed historical reactions reappear. The aux filters must key on `#e`
// only.
test("buildChannelReactionAuxFilter keys on #e only, kind:7, no #h", () => {
  const filter = buildChannelReactionAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.deepEqual(filter.kinds, [7]);
  assert.equal("#h" in filter, false);
});

// The structural overlay (edits/deletions) is the slow half — fetched on its
// own REQ so a stale kind:5 scan can't strand reactions. It must NOT include
// kind:7, or it would re-bundle reactions back into the slow query.
test("buildChannelStructuralAuxFilter keys on #e only, edits+deletions, no kind:7", () => {
  const filter = buildChannelStructuralAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal(filter.kinds.includes(7), false);
  assert.deepEqual(new Set(filter.kinds), new Set([40003, 5, 9005]));
  assert.equal("#h" in filter, false);
});

test("buildChannelAuxDeletionFilter keys on #e only, no #h", () => {
  const filter = buildChannelAuxDeletionFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});
