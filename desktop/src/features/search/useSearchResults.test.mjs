import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveAuthorFromOperator,
  resolveChannelIdFromOperator,
} from "./useSearchResults.ts";

const ELODIE = "e".repeat(64);
const JEROME = "f".repeat(64);

test("from:@name resolves display names ignoring accents on both sides", () => {
  const candidates = [
    { pubkey: ELODIE, displayName: "Élodie" },
    { pubkey: JEROME, displayName: "Jerome" },
  ];
  for (const [raw, expected] of [
    ["@elodie", ELODIE],
    ["ÉLODIE", ELODIE],
    ["@jérôme", JEROME],
    [JEROME.toUpperCase(), JEROME],
  ]) {
    assert.deepEqual(
      resolveAuthorFromOperator(raw, candidates),
      { status: "resolved", value: expected },
      raw,
    );
  }
  assert.deepEqual(resolveAuthorFromOperator("@eloise", candidates), {
    status: "unresolved",
  });
});

test("in:#channel resolves names and labels ignoring accents on both sides", () => {
  const channels = [
    { id: "chan-reunion", name: "réunion" },
    { id: "chan-equipe", name: "equipe" },
  ];
  for (const [raw, expected] of [
    ["#reunion", "chan-reunion"],
    ["#équipe", "chan-equipe"],
  ]) {
    assert.deepEqual(
      resolveChannelIdFromOperator(raw, channels),
      { status: "resolved", value: expected },
      raw,
    );
  }
  assert.deepEqual(
    resolveChannelIdFromOperator("#cafe", channels, {
      "chan-equipe": "Café",
    }),
    { status: "resolved", value: "chan-equipe" },
  );
});
