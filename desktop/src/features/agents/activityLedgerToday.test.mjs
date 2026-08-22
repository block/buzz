import assert from "node:assert/strict";
import test from "node:test";

import {
  activityLedgerArchiveQueryRange,
  activityLedgerDayRange,
  applyAuthorityToTodayActivity,
  buildBoundedTodayActivitySurface,
  buildTodayActivityFromArchivedEvents,
  buildTodayActivityFromArchivedPages,
} from "./activityLedgerToday.ts";
import { journalVerificationSources } from "./activityLedgerAuthority.ts";

function relayEvent({ id, pubkey = "agent-a", agent = pubkey, decoded }) {
  return {
    id,
    pubkey,
    created_at: Math.floor(Date.parse(decoded.timestamp) / 1000),
    kind: 24200,
    tags: [["agent", agent]],
    content: "encrypted",
    sig: `sig-${id}`,
    decoded,
  };
}

test("Today reconstruction trusts only managed self-authored observer frames", async () => {
  const timestamp = "2026-08-21T14:00:00.000Z";
  const valid = relayEvent({
    id: "valid",
    decoded: {
      seq: 1,
      timestamp,
      kind: "turn_started",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: { triggeringEventIds: ["message-1"] },
    },
  });
  const forged = relayEvent({
    id: "forged",
    pubkey: "attacker",
    agent: "agent-a",
    decoded: { ...valid.decoded, seq: 2 },
  });
  const unknown = relayEvent({
    id: "unknown",
    pubkey: "agent-z",
    decoded: { ...valid.decoded, seq: 3 },
  });

  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [valid, forged, unknown],
    decrypt: async (event) => event.decoded,
  });

  assert.equal(surface.counts.journals, 1);
  assert.equal(surface.journals[0].agentName, "Honey");
  assert.equal(surface.journals[0].events.length, 1);
  assert.deepEqual(surface.journals[0].events[0].provenance, {
    ...surface.journals[0].events[0].provenance,
    sourceEventId: "valid",
    sourcePubkey: "agent-a",
    sourceKind: 24200,
    sourceCreatedAt: valid.created_at,
    sourceSignature: "sig-valid",
    origin: "historical_backfill",
  });
});

test("Today reconstruction skips decrypt failures without admitting bad proof", async () => {
  const event = relayEvent({
    id: "bad-ciphertext",
    decoded: {
      seq: 1,
      timestamp: "2026-08-21T14:00:00.000Z",
      kind: "turn_started",
      payload: {},
    },
  });
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [event],
    decrypt: async () => {
      throw new Error("decrypt failed");
    },
  });
  assert.equal(surface.counts.journals, 0);
  assert.equal(surface.snapshotProjection.excludedObserverFrames, 1);
  assert.equal(surface.snapshotProjection.bounded, true);
});

test("Today reconstruction discloses frames excluded before range paging", async () => {
  const surface = await buildTodayActivityFromArchivedPages({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    pages: [{ events: [], unindexedObserverFrames: 2 }],
  });

  assert.equal(surface.counts.journals, 0);
  assert.equal(surface.snapshotProjection.excludedObserverFrames, 2);
  assert.equal(surface.snapshotProjection.unindexedObserverFrames, 2);
  assert.equal(surface.snapshotProjection.bounded, true);
});

test("Today reconstruction reports malformed or empty observer batches", async () => {
  const event = relayEvent({
    id: "empty-batch",
    decoded: {
      seq: 1,
      timestamp: "2026-08-21T14:00:00.000Z",
      kind: "batch",
      payload: { events: [{ malformed: true }] },
    },
  });
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [event],
    decrypt: async (candidate) => candidate.decoded,
  });
  assert.equal(surface.counts.journals, 0);
  assert.equal(surface.snapshotProjection.excludedObserverFrames, 1);
});

test("Today reconstruction counts malformed members of a partially valid batch", async () => {
  const timestamp = "2026-08-21T14:00:00.000Z";
  const event = relayEvent({
    id: "partial-batch",
    decoded: {
      seq: 2,
      timestamp,
      kind: "batch",
      payload: {
        events: [
          {
            seq: 1,
            timestamp,
            kind: "turn_started",
            agentIndex: 0,
            channelId: "channel-1",
            sessionId: "session-1",
            turnId: "turn-1",
            payload: { triggeringEventIds: ["message-1"] },
          },
          { kind: "turn_error", malformed: true },
        ],
      },
    },
  });
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [event],
    decrypt: async (candidate) => candidate.decoded,
  });

  assert.equal(surface.counts.journals, 1);
  assert.equal(surface.snapshotProjection.excludedObserverFrames, 1);
  assert.equal(surface.snapshotProjection.bounded, true);
});

test("Today reconstruction counts batch members with invalid timestamps", async () => {
  const timestamp = "2026-08-21T14:00:00.000Z";
  const event = relayEvent({
    id: "invalid-time-batch",
    decoded: {
      seq: 3,
      timestamp,
      kind: "batch",
      payload: {
        events: [
          {
            seq: 1,
            timestamp,
            kind: "turn_started",
            agentIndex: 0,
            channelId: "channel-1",
            sessionId: "session-1",
            turnId: "turn-invalid-time",
            payload: {},
          },
          {
            seq: 2,
            timestamp: "not-a-date",
            kind: "turn_error",
            agentIndex: 0,
            channelId: "channel-1",
            sessionId: "session-1",
            turnId: "turn-invalid-time",
            payload: {},
          },
        ],
      },
    },
  });
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [event],
    decrypt: async (candidate) => candidate.decoded,
  });

  assert.equal(surface.counts.journals, 1);
  assert.equal(surface.journals[0].status, "incomplete");
  assert.equal(surface.snapshotProjection.excludedObserverFrames, 1);
  assert.equal(surface.snapshotProjection.bounded, true);
});

test("Today reconstruction expands every inner event from one signed batch", async () => {
  const timestamp = "2026-08-21T14:00:00.000Z";
  const batch = relayEvent({
    id: "batch-frame",
    decoded: {
      seq: 99,
      timestamp,
      kind: "batch",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: {
        events: [
          {
            seq: 1,
            timestamp,
            kind: "turn_started",
            agentIndex: 0,
            channelId: "channel-1",
            sessionId: "session-1",
            turnId: "turn-1",
            payload: { triggeringEventIds: ["message-1"] },
          },
          {
            seq: 2,
            timestamp,
            kind: "turn_completed",
            agentIndex: 0,
            channelId: "channel-1",
            sessionId: "session-1",
            turnId: "turn-1",
            payload: {},
          },
        ],
      },
    },
  });

  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [batch],
    decrypt: async (event) => event.decoded,
  });

  assert.equal(surface.counts.journals, 1);
  assert.equal(surface.journals[0].events.length, 2);
  assert.deepEqual(
    surface.journals[0].events.map((event) => event.provenance.sourceEventId),
    ["batch-frame", "batch-frame"],
  );
});

test("Today reconstruction discards earlier pages after an archive restart", async () => {
  const event = (id, turnId) =>
    relayEvent({
      id,
      decoded: {
        seq: 1,
        timestamp: "2026-08-21T14:00:00.000Z",
        kind: "turn_started",
        turnId,
        payload: {},
      },
    });
  const oldEvent = event("old-frame", "old-turn");
  const currentEvent = event("current-frame", "current-turn");
  const surface = await buildTodayActivityFromArchivedPages({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    pages: [
      {
        events: [oldEvent],
        unindexedObserverFrames: 1,
        archiveRevision: 7,
      },
      {
        events: [],
        unindexedObserverFrames: 0,
        archiveRevision: 8,
        reset: true,
      },
      {
        events: [currentEvent],
        unindexedObserverFrames: 0,
        archiveRevision: 8,
      },
    ],
    decrypt: async (candidate) => candidate.decoded,
  });

  assert.deepEqual(
    surface.journals.map((journal) => journal.journalKey),
    ["current-turn"],
  );
  assert.equal(surface.snapshotProjection.unindexedObserverFrames, 0);
  assert.equal(surface.snapshotProjection.archiveRevision, 8);
});

test("Today archive decryption is concurrency-bounded and page-incremental", async () => {
  let active = 0;
  let maxActive = 0;
  let resumedAfterFirstPage = false;
  const makePage = (offset) =>
    Array.from({ length: 20 }, (_, index) =>
      relayEvent({
        id: `frame-${offset + index}`,
        decoded: {
          seq: offset + index + 1,
          timestamp: `2026-08-21T14:${String(offset + index).padStart(2, "0")}:00.000Z`,
          kind: "turn_started",
          turnId: `turn-${offset + index}`,
          payload: {},
        },
      }),
    );
  async function* pages() {
    yield makePage(0);
    assert.equal(
      active,
      0,
      "the next raw page was requested before decrypt drained",
    );
    resumedAfterFirstPage = true;
    yield makePage(20);
  }

  const surface = await buildTodayActivityFromArchivedPages({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    pages: pages(),
    decrypt: async (event) => {
      active += 1;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setTimeout(resolve, 1));
      active -= 1;
      return event.decoded;
    },
  });

  assert.equal(resumedAfterFirstPage, true);
  assert.equal(maxActive, 8);
  assert.equal(surface.counts.journals, 40);
});

test("Today archive reconstruction bounds decoded history between pages", async () => {
  const totalJournals = 1_000;
  const pageSize = 100;
  const largeClaim = "x".repeat(16 * 1024);
  async function* pages() {
    for (let offset = 0; offset < totalJournals; offset += pageSize) {
      yield Array.from({ length: pageSize }, (_, pageIndex) => {
        const index = offset + pageIndex;
        const timestamp = new Date(
          Date.parse("2026-08-21T14:00:00.000Z") + index * 1_000,
        ).toISOString();
        return relayEvent({
          id: `large-frame-${index}`,
          decoded: {
            seq: index + 1,
            timestamp,
            kind: "acp_read",
            turnId: `large-turn-${index}`,
            payload: {
              method: "session/update",
              params: {
                update: {
                  sessionUpdate: "agent_message_chunk",
                  content: { text: largeClaim },
                },
              },
            },
          },
        });
      });
    }
  }

  const surface = await buildTodayActivityFromArchivedPages({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    pages: pages(),
    decrypt: async (event) => event.decoded,
  });

  const encodedBytes = new TextEncoder().encode(JSON.stringify(surface));
  assert.ok(encodedBytes.byteLength <= 6 * 1024 * 1024);
  assert.equal(surface.snapshotProjection.originalJournals, totalJournals);
  assert.ok(surface.snapshotProjection.omittedJournals > 0);
  assert.ok(surface.journals.length < totalJournals);
  assert.equal(surface.journals.at(-1).id, "large-turn-999");
});

test("Today archive checkpoints preserve long-journal verification sources", async () => {
  const timestamp = (seq) =>
    new Date(
      Date.parse("2026-08-21T14:00:00.000Z") + seq * 1_000,
    ).toISOString();
  const decoded = [
    {
      seq: 1,
      timestamp: timestamp(1),
      kind: "turn_started",
      turnId: "long-turn",
      payload: { triggeringEventIds: ["message-root"] },
    },
    {
      seq: 2,
      timestamp: timestamp(2),
      kind: "acp_read",
      turnId: "long-turn",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: "early-tool",
            title: "write_file",
            status: "completed",
            rawOutput: "written",
          },
        },
      },
    },
    ...Array.from({ length: 400 }, (_, index) => ({
      seq: index + 3,
      timestamp: timestamp(index + 3),
      kind: "acp_read",
      turnId: "long-turn",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { text: `claim-${index}` },
          },
        },
      },
    })),
    {
      seq: 403,
      timestamp: timestamp(403),
      kind: "turn_completed",
      turnId: "long-turn",
      payload: {},
    },
  ];
  const archivedNewestFirst = decoded
    .map((event) =>
      relayEvent({
        id: event.seq.toString(16).padStart(64, "0"),
        decoded: event,
      }),
    )
    .reverse();
  async function* pages() {
    for (let index = 0; index < archivedNewestFirst.length; index += 75) {
      yield archivedNewestFirst.slice(index, index + 75);
    }
  }

  const surface = await buildTodayActivityFromArchivedPages({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    pages: pages(),
    decrypt: async (event) => event.decoded,
  });
  const journal = surface.journals[0];
  const sources = journalVerificationSources(journal);

  assert.equal(journal.eventCount, decoded.length);
  assert.equal(journal.proofState, "RECEIPTED");
  assert.equal(sources.hasReceiptedEvidence, true);
  assert.equal(sources.hasCorrelationEvidence, true);
  assert.ok(sources.sourceEventIds.includes("2".padStart(64, "0")));
});

test("Today archive checkpoints reserve the newest signed frame before receipts", async () => {
  const timestamp = (seq) =>
    new Date(
      Date.parse("2026-08-21T14:00:00.000Z") + seq * 1_000,
    ).toISOString();
  const decoded = [
    {
      seq: 1,
      timestamp: timestamp(1),
      kind: "turn_started",
      turnId: "receipt-heavy-turn",
      payload: {},
    },
    ...Array.from({ length: 300 }, (_, index) => ({
      seq: index + 2,
      timestamp: timestamp(index + 2),
      kind: "acp_read",
      turnId: "receipt-heavy-turn",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: `tool-${index}`,
            title: "write_file",
            status: "completed",
            rawOutput: "written",
          },
        },
      },
    })),
    {
      seq: 302,
      timestamp: timestamp(302),
      kind: "acp_read",
      turnId: "receipt-heavy-turn",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { text: "later journal activity" },
          },
        },
      },
    },
  ];
  const latestSourceId = decoded.at(-1).seq.toString(16).padStart(64, "0");
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: decoded.map((event) =>
      relayEvent({
        id: event.seq.toString(16).padStart(64, "0"),
        decoded: event,
      }),
    ),
    decrypt: async (event) => event.decoded,
  });
  const journal = surface.journals[0];

  assert.equal(journal.eventCount, decoded.length);
  assert.equal(journal.events.length, 300);
  assert.equal(journal.events.at(-1).provenance.sourceEventId, latestSourceId);
  assert.ok(
    journalVerificationSources(journal).sourceEventIds.includes(latestSourceId),
  );
});

test("day range is half-open and rejects impossible dates", () => {
  const range = activityLedgerDayRange("2026-08-21");
  assert.equal(range.endCreatedAt - range.startCreatedAt, 24 * 60 * 60);
  assert.throws(() => activityLedgerDayRange("2026-02-30"));
  assert.throws(() => activityLedgerDayRange("08/21/2026"));
});

test("Today archive query uses the authoritative inner-time day range", async () => {
  const day = "2026-08-21";
  const exact = activityLedgerDayRange(day);
  const query = activityLedgerArchiveQueryRange(day);
  assert.deepEqual(query, exact);

  const innerTimestamp = new Date(
    (exact.endCreatedAt - 1) * 1_000,
  ).toISOString();
  const crossMidnightEnvelope = relayEvent({
    id: "cross-midnight",
    decoded: {
      seq: 1,
      timestamp: innerTimestamp,
      kind: "turn_started",
      turnId: "midnight-turn",
      payload: {},
    },
  });
  crossMidnightEnvelope.created_at = exact.endCreatedAt + 1;

  const previousDay = await buildTodayActivityFromArchivedEvents({
    day,
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [crossMidnightEnvelope],
    decrypt: async (event) => event.decoded,
  });
  assert.equal(previousDay.counts.journals, 1);

  const nextDay = new Date(exact.endCreatedAt * 1_000);
  const nextDayKey = `${nextDay.getFullYear()}-${String(nextDay.getMonth() + 1).padStart(2, "0")}-${String(nextDay.getDate()).padStart(2, "0")}`;
  const followingDay = await buildTodayActivityFromArchivedEvents({
    day: nextDayKey,
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [crossMidnightEnvelope],
    decrypt: async (event) => event.decoded,
  });
  assert.equal(followingDay.counts.journals, 0);
});

test("Today authority overlay recomputes evidence-gap counts", async () => {
  const agentPubkey = "1".repeat(64);
  const event = relayEvent({
    id: "a".repeat(64),
    pubkey: agentPubkey,
    decoded: {
      seq: 1,
      timestamp: "2026-08-21T14:00:00.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: {},
    },
  });
  const ended = relayEvent({
    id: "b".repeat(64),
    pubkey: agentPubkey,
    decoded: { ...event.decoded, seq: 2, kind: "turn_completed" },
  });
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: agentPubkey, name: "Honey" }],
    events: [event, ended],
    decrypt: async (candidate) => candidate.decoded,
  });
  const journal = surface.journals[0];
  const updated = applyAuthorityToTodayActivity(
    surface,
    [
      {
        ownerPubkey: "owner-a",
        relayUrl: "wss://relay.example",
        agentPubkey,
        eventId: "c".repeat(64),
        signature: "owner-signature",
        createdAt: ended.created_at + 1,
        artifactType: "verification",
        journalId: journal.id,
        correlationId: journal.correlationId,
        revision: 1,
        summary: null,
        note: null,
        receiptRef: "receipt:owner-check",
        sourceEventIds: [event.id, ended.id].sort(),
      },
    ],
    "wss://relay.example",
  );

  assert.equal(updated.journals[0].proofState, "VERIFIED");
  assert.equal(updated.counts.claimedWithoutEvidence, 0);
  assert.equal(updated.channels[0].lastActivityAt, "2026-08-21T14:00:01.000Z");
});

function snapshotJournal(id, minute, detail = null) {
  const timestamp = `2026-08-21T14:${String(minute).padStart(2, "0")}:00.000Z`;
  const event = {
    id: `event-${id}`,
    journalKey: id,
    correlationId: `message-${id}`,
    category: "tool",
    title: "write_file",
    detail,
    status: "completed",
    proofState: "RECEIPTED",
    timestamp,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: id,
    toolCallId: `tool-${id}`,
    messageId: null,
    provenance: {
      sourceEventId: `source-${id}`,
      sourcePubkey: "agent-a",
      sourceKind: 24200,
      sourceCreatedAt: Math.floor(Date.parse(timestamp) / 1_000),
      sourceSignature: "a".repeat(128),
      origin: "historical_backfill",
      observerKind: "acp_read",
      method: "session/update",
      sessionUpdate: "tool_call_update",
      seq: minute,
      timestamp,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: id,
      toolCallId: `tool-${id}`,
      messageId: null,
      triggeringEventIds: [`message-${id}`],
    },
    tags: ["tool", "tool:write_file"],
  };
  return {
    id,
    journalKey: id,
    correlationId: `message-${id}`,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: id,
    startedAt: timestamp,
    endedAt: timestamp,
    status: "completed",
    proofState: "RECEIPTED",
    summary: `Completed ${id}`,
    summarySource: "auto",
    ownerModifiedAt: null,
    ownerModifiedBy: null,
    claimedCompletionWithoutEvidence: false,
    eventCount: 1,
    events: [event],
    agentPubkey: "agent-a",
    agentName: "Honey",
  };
}

test("Today snapshot projection bounds oversized tool output", () => {
  const journal = snapshotJournal(
    "turn-large",
    1,
    "x".repeat(10 * 1024 * 1024),
  );
  const surface = {
    day: "2026-08-21",
    journals: [journal],
    channels: [],
    counts: {
      journals: 1,
      failed: 0,
      inProgress: 0,
      claimedWithoutEvidence: 0,
    },
  };
  const maxBytes = 32 * 1024;
  const bounded = buildBoundedTodayActivitySurface(surface, maxBytes);

  assert.ok(
    new TextEncoder().encode(JSON.stringify(bounded)).byteLength <= maxBytes,
  );
  assert.equal(bounded.journals.length, 1);
  assert.equal(bounded.snapshotProjection.bounded, true);
  assert.equal(bounded.snapshotProjection.textFieldsTruncated, 1);
  assert.ok(bounded.journals[0].events[0].detail.length < 10 * 1024 * 1024);
});

test("Today snapshot projection drops oldest journals only as a final fallback", () => {
  const journals = Array.from({ length: 10 }, (_, index) =>
    snapshotJournal(`turn-${index}`, index),
  );
  const surface = {
    day: "2026-08-21",
    journals,
    channels: [],
    counts: {
      journals: journals.length,
      failed: 0,
      inProgress: 0,
      claimedWithoutEvidence: 0,
    },
  };
  const maxBytes = 2_500;
  const bounded = buildBoundedTodayActivitySurface(surface, maxBytes);

  assert.ok(
    new TextEncoder().encode(JSON.stringify(bounded)).byteLength <= maxBytes,
  );
  assert.ok(bounded.journals.length > 0);
  assert.ok(bounded.journals.length < journals.length);
  assert.equal(bounded.journals.at(-1).id, "turn-9");
  assert.deepEqual(
    bounded.journals.map((journal) => journal.id),
    journals.slice(-bounded.journals.length).map((journal) => journal.id),
  );
  assert.equal(
    bounded.snapshotProjection.omittedJournals,
    journals.length - bounded.journals.length,
  );
});

test("Today snapshot compaction retains owner verification before receipt overflow", () => {
  const base = snapshotJournal("verified-long", 1);
  const receipts = Array.from({ length: 150 }, (_, index) => ({
    ...base.events[0],
    id: `receipt-${index}`,
    proofState: "VERIFIED",
    provenance: {
      ...base.events[0].provenance,
      sourceEventId: index.toString(16).padStart(64, "0"),
      seq: index + 1,
    },
  }));
  const verification = {
    ...base.events[0],
    id: "owner-verification",
    title: "Owner verification",
    proofState: "VERIFIED",
    provenance: {
      ...base.events[0].provenance,
      sourceEventId: "f".repeat(64),
      sourceKind: 24201,
      observerKind: "owner_verification",
      seq: 151,
    },
  };
  const journal = {
    ...base,
    proofState: "VERIFIED",
    eventCount: receipts.length + 1,
    events: [...receipts, verification],
  };
  const bounded = buildBoundedTodayActivitySurface({
    day: "2026-08-21",
    journals: [journal],
    channels: [],
    counts: {
      journals: 1,
      failed: 0,
      inProgress: 0,
      claimedWithoutEvidence: 0,
    },
  });

  assert.equal(bounded.journals[0].events.length, 100);
  assert.equal(
    bounded.journals[0].events.some(
      (event) => event.provenance.observerKind === "owner_verification",
    ),
    true,
  );
});
