import assert from "node:assert/strict";
import test from "node:test";

import {
  advanceProjectChannelRequestQueue,
  createProjectChannelRequestQueue,
  enqueueProjectChannelRequest,
  MAX_PENDING_PROJECT_CHANNEL_REQUESTS,
} from "./projectChannelRequestQueue.ts";

function candidate(requestId) {
  return {
    agentPubkey: `agent-${requestId}`,
    request: { requestId },
  };
}

test("accepted requests advance in order while duplicate active requests stay suppressed", () => {
  const queue = createProjectChannelRequestQueue();
  const first = candidate("request-a");
  const second = candidate("request-b");

  assert.deepEqual(enqueueProjectChannelRequest(queue, first), {
    status: "show",
    candidate: first,
  });
  assert.deepEqual(enqueueProjectChannelRequest(queue, second), {
    status: "queued",
  });
  assert.deepEqual(enqueueProjectChannelRequest(queue, first), {
    status: "duplicate",
  });
  assert.equal(advanceProjectChannelRequestQueue(queue), second);
  assert.deepEqual(enqueueProjectChannelRequest(queue, first), {
    status: "duplicate",
  });
  assert.equal(advanceProjectChannelRequestQueue(queue), null);
});

test("accepted queue drops newest overflow while preserving FIFO and retryability", () => {
  const queue = createProjectChannelRequestQueue();
  const first = candidate("request-0");
  assert.equal(enqueueProjectChannelRequest(queue, first).status, "show");

  const pending = Array.from(
    { length: MAX_PENDING_PROJECT_CHANNEL_REQUESTS },
    (_, index) => candidate(`request-${index + 1}`),
  );
  for (const request of pending) {
    assert.equal(enqueueProjectChannelRequest(queue, request).status, "queued");
  }

  const overflow = candidate("request-overflow");
  assert.deepEqual(enqueueProjectChannelRequest(queue, overflow), {
    status: "overflow",
  });
  assert.equal(queue.pending.length, MAX_PENDING_PROJECT_CHANNEL_REQUESTS);
  assert.equal(queue.seenRequestIds.has(overflow.request.requestId), false);

  for (const request of pending) {
    assert.equal(advanceProjectChannelRequestQueue(queue), request);
  }
  assert.equal(advanceProjectChannelRequestQueue(queue), null);
  assert.deepEqual(enqueueProjectChannelRequest(queue, overflow), {
    status: "show",
    candidate: overflow,
  });
});

test("dedup history stays bounded without forgetting active or pending requests", () => {
  const queue = createProjectChannelRequestQueue();
  for (let index = 0; index < 500; index += 1) {
    const request = candidate(`request-${index}`);
    assert.equal(enqueueProjectChannelRequest(queue, request).status, "show");
    assert.deepEqual(enqueueProjectChannelRequest(queue, request), {
      status: "duplicate",
    });
    assert.equal(advanceProjectChannelRequestQueue(queue), null);
  }

  assert.ok(queue.seenRequestIds.size <= 201);
});
