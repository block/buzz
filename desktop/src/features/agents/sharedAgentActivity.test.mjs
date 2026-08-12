import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";

import {
  buildAgentActivitySummaryFilter,
  describeSharedAgentActivity,
  mergeSharedAgentActivities,
  parseAgentActivityEvent,
  resolveAgentActivityMode,
} from "./sharedAgentActivity.ts";

const SECRET = new Uint8Array(32).fill(7);
const AGENT = getPublicKey(SECRET);
const OTHER_AGENT = getPublicKey(new Uint8Array(32).fill(8));
const VIEWER = "33".repeat(32);
const OWNER = "44".repeat(32);
const CHANNEL = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const OTHER_CHANNEL = "fba766b8-ecb5-4b04-8ec4-6d82fbd644ac";
const NOW = 1_800_000_000;

function activity(overrides = {}) {
  return {
    activityId: "15ee77b4-92f8-4cb7-9851-80c3d828b62c",
    occurredAt: "2027-01-15T08:00:00Z",
    activityClass: "tool",
    status: "running",
    toolKind: "search",
    ...overrides,
  };
}

function signedEvent({
  content = JSON.stringify({ version: 1, activities: [activity()] }),
  createdAt = NOW,
  tags = [
    ["h", CHANNEL],
    ["agent", AGENT],
  ],
} = {}) {
  return finalizeEvent(
    { kind: 24_201, created_at: createdAt, tags, content },
    SECRET,
  );
}

function parse(event, overrides = {}) {
  return parseAgentActivityEvent(event, {
    expectedAgentPubkey: AGENT,
    expectedChannelId: CHANNEL,
    nowSeconds: NOW,
    ...overrides,
  });
}

test("accepts an exact signed, fresh, channel-bound activity frame", () => {
  assert.deepEqual(parse(signedEvent()), {
    version: 1,
    activities: [activity()],
  });
});

test("rejects tampering, another signer, another channel, and stale frames", () => {
  const tampered = signedEvent();
  tampered.content = JSON.stringify({
    version: 1,
    activities: [activity({ toolKind: "edit" })],
  });
  assert.equal(parse(tampered), null);
  assert.equal(
    parse(signedEvent(), { expectedAgentPubkey: OTHER_AGENT }),
    null,
  );
  assert.equal(
    parse(
      signedEvent({
        tags: [
          ["h", OTHER_CHANNEL],
          ["agent", AGENT],
        ],
      }),
    ),
    null,
  );
  assert.equal(parse(signedEvent({ createdAt: NOW - 301 })), null);
  assert.equal(parse(signedEvent({ createdAt: NOW + 301 })), null);
});

test("rejects duplicate, malformed, extended, and unexpected tags", () => {
  for (const tags of [
    [
      ["h", CHANNEL],
      ["h", CHANNEL],
      ["agent", AGENT],
    ],
    [["h", CHANNEL], ["agent"]],
    [
      ["h", CHANNEL, "extra"],
      ["agent", AGENT],
    ],
    [
      ["h", CHANNEL],
      ["agent", AGENT],
      ["p", VIEWER],
    ],
  ])
    assert.equal(parse(signedEvent({ tags })), null);
});

test("rejects oversized and open-schema content before admission", () => {
  assert.equal(parse(signedEvent({ content: "x".repeat(4_097) })), null);
  assert.equal(
    parse(
      signedEvent({
        content: JSON.stringify({
          version: 1,
          activities: [activity({ detail: "private path" })],
        }),
      }),
    ),
    null,
  );
  assert.equal(
    parse(
      signedEvent({
        content: JSON.stringify({
          version: 1,
          activities: [activity({ toolKind: "unknown-tool" })],
        }),
      }),
    ),
    null,
  );
});

test("enforces closed class/status/field combinations", () => {
  const invalid = [
    activity({ activityClass: "turn", status: "pending", toolKind: undefined }),
    activity({ activityClass: "turn", status: "running" }),
    activity({ activityClass: "tool", status: "started" }),
    activity({
      activityClass: "usage",
      status: "running",
      toolKind: undefined,
      usage: { totalTokens: 5 },
    }),
    activity({
      activityClass: "usage",
      status: "completed",
      toolKind: undefined,
      usage: {},
    }),
    activity({ status: "running", durationMs: 10 }),
  ];
  for (const item of invalid) {
    assert.equal(
      parse(
        signedEvent({
          content: JSON.stringify({ version: 1, activities: [item] }),
        }),
      ),
      null,
    );
  }
});

test("rejects explicit null for every optional field", () => {
  for (const field of ["toolKind", "durationMs", "usage"]) {
    assert.equal(
      parse(
        signedEvent({
          content: JSON.stringify({
            version: 1,
            activities: [activity({ [field]: null })],
          }),
        }),
      ),
      null,
      field,
    );
  }
  for (const field of [
    "inputTokens",
    "outputTokens",
    "totalTokens",
    "cacheReadTokens",
    "cacheWriteTokens",
  ]) {
    assert.equal(
      parse(
        signedEvent({
          content: JSON.stringify({
            version: 1,
            activities: [
              activity({
                activityClass: "usage",
                status: "completed",
                toolKind: undefined,
                usage: { [field]: null },
              }),
            ],
          }),
        }),
      ),
      null,
      field,
    );
  }
});

test("merges lifecycle updates by opaque id and keeps a bounded newest window", () => {
  const first = activity({ occurredAt: "2027-01-15T08:00:00Z" });
  const completed = activity({
    occurredAt: "2027-01-15T08:00:03Z",
    status: "completed",
    durationMs: 3_000,
  });
  const second = activity({
    activityId: "deabf692-9cb3-4d41-ad71-b915d2477fea",
    occurredAt: "2027-01-15T08:00:02Z",
  });
  assert.deepEqual(
    mergeSharedAgentActivities([first], [second, completed], 2),
    [second, completed],
  );
  assert.deepEqual(mergeSharedAgentActivities([first], [second], 1), [second]);
});

test("owner mode requires exact verified profile ownership", () => {
  assert.equal(
    resolveAgentActivityMode({
      agentOwnerPubkey: OWNER,
      currentPubkey: OWNER.toUpperCase(),
      channel: null,
    }),
    "owner",
  );
  assert.equal(
    resolveAgentActivityMode({
      agentOwnerPubkey: null,
      currentPubkey: OWNER,
      channel: { id: CHANNEL, channelType: "stream", isMember: true },
    }),
    "shared",
  );
  assert.equal(
    resolveAgentActivityMode({
      agentOwnerPubkey: VIEWER,
      currentPubkey: OWNER,
      channel: { id: CHANNEL, channelType: "forum", isMember: true },
    }),
    "shared",
  );
});

test("shared mode is limited to current stream/forum members", () => {
  for (const channelType of ["stream", "forum"]) {
    assert.equal(
      resolveAgentActivityMode({
        agentOwnerPubkey: OWNER,
        currentPubkey: VIEWER,
        channel: { id: CHANNEL, channelType, isMember: true },
      }),
      "shared",
    );
  }
  for (const channel of [
    { id: CHANNEL, channelType: "dm", isMember: true },
    { id: CHANNEL, channelType: "stream", isMember: false },
    null,
  ]) {
    assert.equal(
      resolveAgentActivityMode({
        agentOwnerPubkey: OWNER,
        currentPubkey: VIEWER,
        channel,
      }),
      "unavailable",
    );
  }
});

test("summary subscription uses one exact channel and author with no history", () => {
  assert.deepEqual(buildAgentActivitySummaryFilter(AGENT, CHANNEL), {
    kinds: [24_201],
    authors: [AGENT],
    "#h": [CHANNEL],
    limit: 0,
  });
});

test("internal planning is rendered neutrally and never exposed as reasoning", () => {
  const description = describeSharedAgentActivity(
    activity({ toolKind: "think", status: "running" }),
  );
  assert.deepEqual(description, { label: "Working", detail: "In progress" });
  assert.doesNotMatch(
    `${description.label} ${description.detail}`,
    /think|thought|reason|chain/i,
  );
});
