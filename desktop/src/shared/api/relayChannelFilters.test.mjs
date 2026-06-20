import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAuxDeletionFilter,
  buildChannelAuxFilter,
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
test("buildChannelAuxFilter keys on #e only, no #h", () => {
  const filter = buildChannelAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelAuxDeletionFilter keys on #e only, no #h", () => {
  const filter = buildChannelAuxDeletionFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});
