import assert from "node:assert/strict";
import test from "node:test";

import { truncatePubkey } from "@/shared/lib/pubkey";

import {
  getPersistentMentionTokenRemovalRange,
  resolvePersistentMentionTargets,
} from "./usePersistentAgentMentionHydration.ts";

const agentA = "a".repeat(64);
const agentB = "b".repeat(64);

test("persistent hydration gives duplicate agent names identity-safe mention text", () => {
  const targets = resolvePersistentMentionTargets(
    [agentA, agentB],
    () => "Morgarita",
  );

  assert.deepEqual(targets, [
    { pubkey: agentA, displayName: `Morgarita (${truncatePubkey(agentA)})` },
    { pubkey: agentB, displayName: `Morgarita (${truncatePubkey(agentB)})` },
  ]);
});

test("persistent mention removal targets the duplicate-name pubkey's exact label", () => {
  const targets = resolvePersistentMentionTargets(
    [agentA, agentB],
    () => "Morgarita",
  );
  const hydratedLabels = new Map(
    targets.map((target) => [target.pubkey, target.displayName]),
  );
  const text = `${targets.map((target) => `@${target.displayName}`).join(" ")} `;

  const range = getPersistentMentionTokenRemovalRange(
    text,
    agentB,
    hydratedLabels,
    () => "Morgarita",
  );

  assert.deepEqual(range, {
    from: `@${targets[0].displayName} `.length,
    to: text.length,
  });
  assert.equal(
    text.slice(0, range.from) + text.slice(range.to),
    `@${targets[0].displayName} `,
  );
});
