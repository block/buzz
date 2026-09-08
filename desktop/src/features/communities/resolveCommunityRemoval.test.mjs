import assert from "node:assert/strict";
import test from "node:test";

import { resolveCommunityRemoval } from "./useCommunities.tsx";

const alpha = { id: "alpha", name: "Alpha", relayUrl: "wss://alpha" };
const beta = { id: "beta", name: "Beta", relayUrl: "wss://beta" };

test("removing the final community clears the active community", () => {
  assert.deepEqual(resolveCommunityRemoval([alpha], "alpha", "alpha"), {
    communities: [],
    activeId: null,
  });
});

test("removing the active community selects a clean fallback", () => {
  assert.deepEqual(resolveCommunityRemoval([alpha, beta], "alpha", "alpha"), {
    communities: [beta],
    activeId: "beta",
  });
});

test("removing an inactive community preserves the active community", () => {
  assert.deepEqual(resolveCommunityRemoval([alpha, beta], "alpha", "beta"), {
    communities: [alpha],
    activeId: "alpha",
  });
});
