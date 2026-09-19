import assert from "node:assert/strict";
import test from "node:test";

import { workflowAuthorLookups } from "./useWorkflowListAuthorPresentations.ts";

function workflow(index, pubkey) {
  return {
    id: `workflow-${index}`,
    definition: {
      trigger: {
        on: "message_posted",
        filter: `trigger_author == "${pubkey}"`,
      },
      steps: [],
    },
  };
}

test("collects configured authors for one list-level batch", () => {
  const authors = ["a".repeat(64), "b".repeat(64), "a".repeat(64)];
  const lookups = authors.flatMap((pubkey, index) =>
    workflowAuthorLookups([workflow(index, pubkey)]),
  );

  assert.equal(lookups.length, 3);
  assert.deepEqual(
    [...new Set(lookups.map(({ pubkey }) => pubkey))],
    authors.slice(0, 2),
  );
});
