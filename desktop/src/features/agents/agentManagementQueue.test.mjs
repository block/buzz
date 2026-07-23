import assert from "node:assert/strict";
import test from "node:test";

import {
  placeAcceptedRequest,
  takeNextQueuedRequest,
} from "./agentManagementQueue.ts";

const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);

function createRequest(requestId, displayName) {
  return {
    type: "agent_management_request",
    action: "create",
    requestId,
    request: {
      channelId: "channel-1",
      displayName,
      systemPrompt: "prompt",
    },
  };
}

test("shows the first draft immediately when nothing is pending", () => {
  assert.equal(placeAcceptedRequest(false), "show");
});

test("enqueues a draft that arrives while one is already pending", () => {
  assert.equal(placeAcceptedRequest(true), "enqueue");
});

test("drains queued drafts first-in-first-out", () => {
  const queue = [
    { agentPubkey: AGENT_A, request: createRequest("r1", "First") },
    { agentPubkey: AGENT_B, request: createRequest("r2", "Second") },
  ];

  const first = takeNextQueuedRequest(queue);
  assert.equal(first?.request.requestId, "r1");
  assert.equal(first?.agentPubkey, AGENT_A);

  const second = takeNextQueuedRequest(queue);
  assert.equal(second?.request.requestId, "r2");
  assert.equal(second?.agentPubkey, AGENT_B);

  assert.equal(queue.length, 0);
});

test("returns undefined when the queue is empty", () => {
  assert.equal(takeNextQueuedRequest([]), undefined);
});

test("each queued draft keeps its own source pubkey after the queue advances", () => {
  // Regression: the origin check on submit must validate the agent that
  // authored the draft currently shown, so the pubkey must ride with the draft
  // rather than being overwritten by a later arrival.
  const queue = [];

  // Two concurrent drafts arrive while a first draft is already pending.
  assert.equal(placeAcceptedRequest(true), "enqueue");
  queue.push({ agentPubkey: AGENT_A, request: createRequest("r1", "First") });
  assert.equal(placeAcceptedRequest(true), "enqueue");
  queue.push({ agentPubkey: AGENT_B, request: createRequest("r2", "Second") });

  assert.equal(takeNextQueuedRequest(queue)?.agentPubkey, AGENT_A);
  assert.equal(takeNextQueuedRequest(queue)?.agentPubkey, AGENT_B);
});
