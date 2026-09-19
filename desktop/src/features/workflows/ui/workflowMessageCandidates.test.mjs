import assert from "node:assert/strict";
import { test } from "node:test";

import {
  enrichMessageCandidateFromExactLookup,
  isPickableWorkflowMessageEvent,
  mergeMessageCandidateSources,
  normalizeMessageEventId,
  validateWorkflowMessageSearchResults,
  validatedWorkflowMessageCandidate,
} from "./workflowMessageCandidates.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);
const CHANNEL = "workflow-channel";

function relayEvent({
  id = A,
  kind = 9,
  channelId = CHANNEL,
  content = "Fetched preview",
} = {}) {
  return {
    id,
    pubkey: C,
    created_at: 1_725_000_000,
    kind,
    tags: channelId === null ? [] : [["h", channelId]],
    content,
    sig: "",
  };
}

function fallback(id = A) {
  return { id, pubkey: null, content: null, createdAt: null };
}

test("normalizes only valid 64-character hex event IDs", () => {
  assert.equal(normalizeMessageEventId(`  ${A.toUpperCase()}  `), A);
  assert.equal(normalizeMessageEventId(A.slice(1)), null);
  assert.equal(normalizeMessageEventId("z".repeat(64)), null);
});

test("merges valid candidates with stable first-wins source priority", () => {
  const merged = mergeMessageCandidateSources([
    [
      { id: B.toUpperCase(), content: "Recent B" },
      { id: "invalid", content: "Invalid" },
      { id: A, content: "Recent A", pubkey: C },
    ],
    [
      { id: A.toUpperCase(), content: "Search A" },
      { id: C, content: "Search C" },
      { id: B, content: "Search B" },
    ],
  ]);

  assert.deepEqual(
    merged.map(({ id, content }) => ({ id, content })),
    [
      { id: B, content: "Recent B" },
      { id: A, content: "Recent A" },
      { id: C, content: "Search C" },
    ],
  );
  assert.equal(merged[1].pubkey, C);
});

test("accepts only the existing narrow message kinds in the exact channel", () => {
  for (const kind of [9, 40_002, 45_001, 45_003, 40_008]) {
    assert.equal(
      isPickableWorkflowMessageEvent(relayEvent({ kind }), CHANNEL),
      true,
    );
  }

  assert.equal(
    isPickableWorkflowMessageEvent(relayEvent({ kind: 40_099 }), CHANNEL),
    false,
  );
  assert.equal(
    isPickableWorkflowMessageEvent(
      relayEvent({ channelId: "another-channel" }),
      CHANNEL,
    ),
    false,
  );
  assert.equal(
    isPickableWorkflowMessageEvent(
      {
        ...relayEvent(),
        tags: [
          ["h", CHANNEL],
          ["h", "another-channel"],
        ],
      },
      CHANNEL,
    ),
    false,
  );
  assert.equal(
    isPickableWorkflowMessageEvent(relayEvent({ channelId: null }), CHANNEL),
    false,
  );
});

test("single validator rejects wrong IDs, channels, and kinds", () => {
  assert.deepEqual(
    validatedWorkflowMessageCandidate(relayEvent(), {
      channelId: CHANNEL,
      requestedId: A.toUpperCase(),
    }),
    {
      id: A,
      pubkey: C,
      content: "Fetched preview",
      createdAt: 1_725_000_000,
    },
  );
  assert.equal(
    validatedWorkflowMessageCandidate(relayEvent({ id: B }), {
      channelId: CHANNEL,
      requestedId: A,
    }),
    null,
  );
  assert.equal(
    validatedWorkflowMessageCandidate(
      relayEvent({ channelId: "another-channel" }),
      { channelId: CHANNEL },
    ),
    null,
  );
  assert.equal(
    validatedWorkflowMessageCandidate(relayEvent({ kind: 40_099 }), {
      channelId: CHANNEL,
    }),
    null,
  );
});

test("exact lookup enriches only the normalized requested event ID", () => {
  const enriched = enrichMessageCandidateFromExactLookup(
    fallback(A.toUpperCase()),
    relayEvent({ id: A.toUpperCase() }),
    CHANNEL,
  );

  assert.deepEqual(enriched, {
    id: A,
    pubkey: C,
    content: "Fetched preview",
    createdAt: 1_725_000_000,
  });

  const wrongEvent = relayEvent({ id: B, content: "Wrong preview" });
  const original = fallback(A);
  assert.strictEqual(
    enrichMessageCandidateFromExactLookup(original, wrongEvent, CHANNEL),
    original,
  );
});

test("search discovery exposes only exact events that pass the real event boundary", () => {
  const candidates = validateWorkflowMessageSearchResults(
    [
      {
        requestedId: A,
        event: relayEvent({ id: A, content: "Validated search preview" }),
      },
      {
        requestedId: B,
        event: {
          ...relayEvent({ id: B, content: "Projection-laundered preview" }),
          tags: [
            ["h", CHANNEL],
            ["h", "another-channel"],
          ],
        },
      },
      {
        requestedId: C,
        event: relayEvent({ id: B, content: "Wrong exact event" }),
      },
    ],
    CHANNEL,
  );

  assert.deepEqual(candidates, [
    {
      id: A,
      pubkey: C,
      content: "Validated search preview",
      createdAt: 1_725_000_000,
    },
  ]);
});

test("invalid lookup results preserve the deterministic fallback without preview", () => {
  const original = fallback(A);
  const invalidEvents = [
    undefined,
    relayEvent({ channelId: "another-channel", content: "Cross-channel" }),
    relayEvent({ kind: 40_099, content: "System message" }),
    relayEvent({ channelId: null, content: "Unscoped" }),
  ];

  for (const event of invalidEvents) {
    const result = enrichMessageCandidateFromExactLookup(
      original,
      event,
      CHANNEL,
    );
    assert.strictEqual(result, original);
    assert.equal(result.content, null);
  }
});
