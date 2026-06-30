import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAgentConversationMarkerFilter,
  buildChannelAuxDeletionFilter,
  buildChannelAuxFilter,
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

test("buildChannelReactionAuxFilter fetches only kind:7 by #e", () => {
  const filter = buildChannelReactionAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter.kinds, [7]);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelStructuralAuxFilter excludes reactions", () => {
  const filter = buildChannelStructuralAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter.kinds, [5, 9005, 40003]);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelAgentConversationMarkerFilter scopes task markers by channel and references", () => {
  const filter = buildChannelAgentConversationMarkerFilter(CHANNEL, IDS);
  assert.deepEqual(filter.kinds, [40004, 40010]);
  assert.deepEqual(filter["#h"], [CHANNEL]);
  assert.deepEqual(filter["#e"], IDS);
});
