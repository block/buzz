import assert from "node:assert/strict";
import test from "node:test";

import {
  loadWorkflowMessagePresentations,
  workflowMessageLookups,
} from "./useWorkflowListMessagePresentations.ts";

const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";

function workflow(index) {
  const messageId = index.toString(16).padStart(64, "0");
  return {
    id: `workflow-${index}`,
    channelId: CHANNEL_ID,
    definition: {
      trigger: {
        on: "reaction_added",
        filter: `trigger_message_id == "${messageId}"`,
      },
      steps: [],
    },
  };
}

test("loads presentation for many workflow cards with one batched fetch", async () => {
  const workflows = Array.from({ length: 40 }, (_, index) =>
    workflow(index + 1),
  );
  const lookups = workflowMessageLookups(workflows);
  const calls = [];

  const presentations = await loadWorkflowMessagePresentations(
    lookups,
    async (eventIds) => {
      calls.push(eventIds);
      return eventIds.map((id, index) => ({
        id,
        pubkey: "a".repeat(64),
        created_at: index,
        kind: 9,
        tags: [["h", CHANNEL_ID]],
        content: `Message ${index + 1}`,
        sig: "b".repeat(128),
      }));
    },
  );

  assert.equal(calls.length, 1);
  assert.equal(calls[0].length, workflows.length);
  assert.equal(presentations.size, workflows.length);
  assert.equal(presentations.get("workflow-40")?.messageLabel, "Message 40");
});

test("deduplicates shared message IDs inside the single batch", async () => {
  const duplicate = workflow(1);
  duplicate.id = "workflow-duplicate";
  const lookups = workflowMessageLookups([workflow(1), duplicate]);
  let requestedIds = [];

  await loadWorkflowMessagePresentations(lookups, async (eventIds) => {
    requestedIds = eventIds;
    return [];
  });

  assert.deepEqual(requestedIds, [`${"0".repeat(63)}1`]);
});
