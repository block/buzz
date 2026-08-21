import assert from "node:assert/strict";
import test from "node:test";

import {
  applyOwnerJournalOverride,
  buildMissionJournal,
  buildTodayActivitySurface,
  normalizeActivityEvents,
} from "./activityLedger.ts";

const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const CHANNEL_A = "11111111-1111-1111-1111-111111111111";
const CHANNEL_B = "22222222-2222-2222-2222-222222222222";

function observerEvent(overrides = {}) {
  return {
    seq: 1,
    timestamp: "2026-08-21T14:00:00.000Z",
    kind: "turn_started",
    agentIndex: 0,
    channelId: CHANNEL_A,
    sessionId: "sess-1",
    turnId: "turn-1",
    payload: { source: "channel", triggeringEventIds: ["f".repeat(64)] },
    sourceEventId: (overrides.seq ?? 1).toString(16).padStart(64, "0"),
    ...overrides,
  };
}

function sessionUpdate(seq, sessionUpdate, update = {}, eventOverrides = {}) {
  return observerEvent({
    ...eventOverrides,
    seq,
    sourceEventId:
      eventOverrides.sourceEventId ?? seq.toString(16).padStart(64, "0"),
    kind: "acp_read",
    payload: {
      method: "session/update",
      params: {
        sessionId: eventOverrides.sessionId ?? "sess-1",
        update: {
          sessionUpdate,
          ...update,
        },
      },
    },
  });
}

test("normalizeActivityEvents marks failed tool updates as FAILED and preserves tool correlation", () => {
  const events = normalizeActivityEvents([
    observerEvent(),
    sessionUpdate(2, "tool_call", {
      toolCallId: "call-1",
      status: "executing",
      title: "shell",
      kind: "shell",
      rawInput: { command: "cargo test" },
    }),
    sessionUpdate(3, "tool_call_update", {
      toolCallId: "call-1",
      status: "failed",
      title: "shell",
      kind: "shell",
      rawInput: { command: "cargo test" },
      rawOutput: "boom",
    }),
  ]);

  const toolUpdate = events.find(
    (event) => event.category === "tool" && event.status === "failed",
  );
  assert.ok(toolUpdate);
  assert.equal(toolUpdate.proofState, "FAILED");
  assert.equal(toolUpdate.correlationId, "call-1");
  assert.equal(toolUpdate.provenance.toolCallId, "call-1");
});

test("buildMissionJournal flags turn completion without supporting evidence", () => {
  const normalized = normalizeActivityEvents([
    observerEvent(),
    observerEvent({
      seq: 2,
      kind: "turn_completed",
      payload: {},
      sourceEventId: "d".repeat(64),
    }),
  ]);

  const journal = buildMissionJournal(normalized);
  assert.equal(journal.status, "ended_unverified");
  assert.equal(journal.proofState, "OBSERVED");
  assert.equal(journal.claimedCompletionWithoutEvidence, true);
  assert.match(journal.summary, /without supporting evidence/i);
});

test("normalizeActivityEvents deduplicates duplicate observer frames", () => {
  const duplicate = sessionUpdate(2, "tool_call", {
    toolCallId: "call-dup",
    status: "executing",
    title: "read_file",
    kind: "read_file",
    rawInput: { path: "Cargo.toml" },
  });
  const events = normalizeActivityEvents([duplicate, duplicate]);
  assert.equal(events.length, 1);
});

test("signed provenance keeps same seq and timestamp with different ids distinct", () => {
  const events = normalizeActivityEvents([
    observerEvent({ sourceEventId: "1".repeat(64) }),
    observerEvent({ sourceEventId: "2".repeat(64) }),
  ]);
  assert.equal(events.length, 2);
});

test("signed batch siblings keep the same outer id without collapsing", () => {
  const sourceEventId = "3".repeat(64);
  const events = normalizeActivityEvents([
    observerEvent({ seq: 1, sourceEventId }),
    observerEvent({
      seq: 2,
      kind: "turn_completed",
      sourceEventId,
    }),
  ]);
  assert.equal(events.length, 2);
  assert.equal(new Set(events.map((event) => event.id)).size, 2);
  assert.deepEqual(
    events.map((event) => event.provenance.sourceEventId),
    [sourceEventId, sourceEventId],
  );
});

test("completed tool output is RECEIPTED but never implicitly VERIFIED", () => {
  const events = normalizeActivityEvents([
    sessionUpdate(2, "tool_call_update", {
      toolCallId: "call-receipt",
      status: "completed",
      title: "shell",
      kind: "shell",
      rawOutput: "ok",
    }),
  ]);
  assert.equal(events[0].proofState, "RECEIPTED");
});

test("assistant done claim does not verify an unconditional turn end", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent(),
      sessionUpdate(2, "agent_message_chunk", {
        messageId: "message-1",
        content: [{ type: "text", text: "Done." }],
      }),
      observerEvent({
        seq: 3,
        kind: "turn_completed",
        payload: {},
        sourceEventId: "3".repeat(64),
      }),
    ]),
  );
  assert.equal(journal.status, "ended_unverified");
  assert.equal(journal.proofState, "OBSERVED");
  assert.equal(journal.claimedCompletionWithoutEvidence, true);
});

test("agent-authored verifier fields cannot mint VERIFIED", () => {
  const events = normalizeActivityEvents([
    observerEvent({
      kind: "proof_verified",
      payload: {
        verified: true,
        verifierPubkey: "9".repeat(64),
        receiptRef: "receipt:independent-1",
      },
    }),
  ]);
  assert.equal(events[0].proofState, "CLAIMED");
});

test("agent-authored journal override cannot rewrite the observed summary", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent(),
      observerEvent({
        seq: 2,
        kind: "journal_override",
        payload: {
          summary: "Owner says complete",
          modifiedBy: "owner",
        },
      }),
    ]),
  );
  assert.equal(journal.summarySource, "auto");
  assert.equal(journal.ownerModifiedBy, null);
  assert.notEqual(journal.summary, "Owner says complete");
});

test("stale started turn is incomplete and UNKNOWN", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([observerEvent()]),
    { asOf: "2026-08-21T14:10:00.000Z", incompleteAfterMs: 60_000 },
  );
  assert.equal(journal.status, "incomplete");
  assert.equal(journal.proofState, "UNKNOWN");
});

test("recent liveness keeps a long-running turn in progress", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent(),
      observerEvent({
        seq: 2,
        timestamp: "2026-08-21T14:09:00.000Z",
        kind: "turn_liveness",
        payload: {},
      }),
    ]),
    { asOf: "2026-08-21T14:10:00.000Z", incompleteAfterMs: 5 * 60_000 },
  );
  assert.equal(journal.status, "in_progress");
  assert.equal(journal.endedAt, "2026-08-21T14:09:00.000Z");
});

test("real managed-agent runtime failure shape fails the journal", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent({
        kind: "managed_agent_runtime_lifecycle",
        payload: {
          pubkey: AGENT_A,
          relayUrl: "wss://relay.example",
          startNonce: "start-1",
          lifecycle: "failed",
          error: "pool wake task failed",
        },
      }),
    ]),
  );
  assert.equal(journal.status, "failed");
  assert.equal(journal.proofState, "FAILED");
  assert.match(journal.summary, /pool wake task failed/);
});

test("a later runtime ready event clears an earlier runtime failure", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent({
        kind: "managed_agent_runtime_lifecycle",
        payload: {
          pubkey: AGENT_A,
          startNonce: "start-1",
          lifecycle: "failed",
          error: "pool wake task failed",
        },
      }),
      observerEvent({
        seq: 2,
        timestamp: "2026-08-21T14:01:00.000Z",
        kind: "managed_agent_runtime_lifecycle",
        payload: {
          pubkey: AGENT_A,
          startNonce: "start-2",
          lifecycle: "ready",
        },
        sourceEventId: "8".repeat(64),
      }),
    ]),
  );
  assert.equal(journal.status, "observed");
  assert.equal(journal.proofState, "OBSERVED");
  assert.doesNotMatch(journal.summary, /pool wake task failed/);
});

test("the real listening to waking to ready lifecycle does not stay in progress", () => {
  const events = ["listening", "waking", "ready"].map((lifecycle, index) =>
    observerEvent({
      seq: index + 1,
      timestamp: `2026-08-21T14:0${index}:00.000Z`,
      kind: "managed_agent_runtime_lifecycle",
      payload: { lifecycle, startNonce: "start-1" },
      sourceEventId: `${index + 1}`.repeat(64),
    }),
  );
  const journal = buildMissionJournal(normalizeActivityEvents(events));
  assert.equal(journal.status, "observed");
  assert.equal(journal.proofState, "OBSERVED");
});

test("a repeated runtime failure reports the latest failure reason", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent({
        kind: "managed_agent_runtime_lifecycle",
        payload: { lifecycle: "failed", error: "old failure" },
      }),
      observerEvent({
        seq: 2,
        timestamp: "2026-08-21T14:01:00.000Z",
        kind: "managed_agent_runtime_lifecycle",
        payload: { lifecycle: "ready" },
        sourceEventId: "8".repeat(64),
      }),
      observerEvent({
        seq: 3,
        timestamp: "2026-08-21T14:02:00.000Z",
        kind: "managed_agent_runtime_lifecycle",
        payload: { lifecycle: "failed", error: "new failure" },
        sourceEventId: "9".repeat(64),
      }),
    ]),
  );
  assert.equal(journal.status, "failed");
  assert.equal(journal.proofState, "FAILED");
  assert.match(journal.summary, /new failure/);
  assert.doesNotMatch(journal.summary, /old failure/);
});

test("successful retry prevents an earlier failed tool from failing the journal", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent(),
      sessionUpdate(2, "tool_call_update", {
        toolCallId: "call-retry",
        status: "failed",
        title: "shell",
        rawOutput: "boom",
      }),
      sessionUpdate(3, "tool_call_update", {
        toolCallId: "call-retry",
        status: "completed",
        title: "shell",
        rawOutput: "ok",
      }),
      observerEvent({
        seq: 4,
        kind: "turn_completed",
        payload: {},
        sourceEventId: "6".repeat(64),
      }),
    ]),
  );
  assert.equal(journal.status, "completed");
  assert.equal(journal.proofState, "RECEIPTED");
});

test("unrecovered tool failure remains FAILED without claiming mission failure", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent(),
      sessionUpdate(2, "tool_call_update", {
        toolCallId: "call-failed",
        status: "failed",
        title: "shell",
        rawOutput: "boom",
      }),
      observerEvent({
        seq: 3,
        kind: "turn_completed",
        payload: {},
        sourceEventId: "7".repeat(64),
      }),
    ]),
  );
  assert.equal(journal.status, "ended_unverified");
  assert.equal(journal.proofState, "FAILED");
});

test("buildMissionJournal reconstructs the same result from a restart replay", () => {
  const raw = [
    observerEvent(),
    sessionUpdate(2, "tool_call", {
      toolCallId: "call-2",
      status: "executing",
      title: "shell",
      kind: "shell",
      rawInput: { command: "pnpm test" },
    }),
    sessionUpdate(3, "tool_call_update", {
      toolCallId: "call-2",
      status: "completed",
      title: "shell",
      kind: "shell",
      rawInput: { command: "pnpm test" },
      rawOutput: "3 passed",
    }),
    observerEvent({
      seq: 4,
      kind: "turn_completed",
      payload: {},
      sourceEventId: "c".repeat(64),
    }),
  ];

  const first = buildMissionJournal(normalizeActivityEvents(raw));
  const replayed = buildMissionJournal(normalizeActivityEvents([...raw]));

  assert.deepEqual(replayed, first);
});

test("buildMissionJournal selects the most recently active overlapping turn", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent({
        turnId: "turn-a",
        timestamp: "2026-08-21T10:00:00.000Z",
        sourceEventId: "a".repeat(64),
      }),
      observerEvent({
        turnId: "turn-b",
        timestamp: "2026-08-21T11:00:00.000Z",
        sourceEventId: "b".repeat(64),
      }),
      observerEvent({
        turnId: "turn-a",
        timestamp: "2026-08-21T12:00:00.000Z",
        kind: "turn_completed",
        payload: {},
        sourceEventId: "c".repeat(64),
      }),
    ]),
  );
  assert.equal(journal.turnId, "turn-a");
  assert.equal(journal.endedAt, "2026-08-21T12:00:00.000Z");
});

test("buildTodayActivitySurface keeps multi-agent handoffs distinct while aggregating one day", () => {
  const feed = buildTodayActivitySurface(
    [
      {
        agentPubkey: AGENT_A,
        agentName: "Fizz",
        events: normalizeActivityEvents([
          observerEvent({
            sessionId: "sess-a",
            turnId: "turn-a",
            channelId: CHANNEL_A,
          }),
          sessionUpdate(
            2,
            "tool_call_update",
            {
              toolCallId: "call-a",
              status: "completed",
              title: "shell",
              kind: "shell",
              rawInput: { command: "cargo test" },
              rawOutput: "ok",
            },
            { sessionId: "sess-a", turnId: "turn-a", channelId: CHANNEL_A },
          ),
        ]),
      },
      {
        agentPubkey: AGENT_B,
        agentName: "Honey",
        events: normalizeActivityEvents([
          observerEvent({
            sessionId: "sess-b",
            turnId: "turn-b",
            channelId: CHANNEL_A,
            timestamp: "2026-08-21T14:05:00.000Z",
          }),
          sessionUpdate(
            2,
            "plan",
            { content: [{ type: "text", text: "Follow up with owner" }] },
            {
              sessionId: "sess-b",
              turnId: "turn-b",
              channelId: CHANNEL_A,
              timestamp: "2026-08-21T14:06:00.000Z",
            },
          ),
        ]),
      },
    ],
    { day: "2026-08-21" },
  );

  assert.equal(feed.journals.length, 2);
  assert.deepEqual(
    feed.journals.map((journal) => journal.correlationId),
    ["f".repeat(64), "f".repeat(64)],
  );
  assert.equal(feed.channels[0].channelId, CHANNEL_A);
  assert.deepEqual(
    feed.channels[0].agentPubkeys.sort(),
    [AGENT_A, AGENT_B].sort(),
  );
});

test("applyOwnerJournalOverride keeps owner edits separate from observed proof", () => {
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      observerEvent(),
      sessionUpdate(2, "tool_call_update", {
        toolCallId: "call-3",
        status: "completed",
        title: "shell",
        kind: "shell",
        rawInput: { command: "npm test" },
        rawOutput: "ok",
      }),
    ]),
  );

  const overridden = applyOwnerJournalOverride(journal, {
    summary: "Owner note: verify with Bumble before closing.",
    modifiedAt: "2026-08-21T15:00:00.000Z",
    modifiedBy: "owner",
  });

  assert.equal(overridden.summarySource, "owner");
  assert.equal(
    overridden.summary,
    "Owner note: verify with Bumble before closing.",
  );
  assert.equal(overridden.proofState, journal.proofState);
});

test("buildTodayActivitySurface filters out work from other local days", () => {
  const feed = buildTodayActivitySurface(
    [
      {
        agentPubkey: AGENT_A,
        agentName: "Fizz",
        events: normalizeActivityEvents([
          observerEvent({
            timestamp: "2026-08-20T23:58:00.000Z",
            channelId: CHANNEL_B,
          }),
          observerEvent({
            seq: 2,
            timestamp: "2026-08-21T15:00:00.000Z",
            channelId: CHANNEL_A,
          }),
        ]),
      },
    ],
    { day: "2026-08-21" },
  );

  assert.equal(feed.journals.length, 1);
  assert.equal(feed.journals[0].channelId, CHANNEL_A);
});
