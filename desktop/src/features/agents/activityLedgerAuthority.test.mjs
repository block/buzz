import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMissionJournal,
  normalizeActivityEvents,
} from "./activityLedger.ts";
import {
  applyValidatedJournalAuthority,
  journalAuthorityCorrelationId,
  journalVerificationSources,
} from "./activityLedgerAuthority.ts";

const sourceId = "a".repeat(64);
const terminalSourceId = "b".repeat(64);
const relayUrl = "wss://relay.example";
const agentPubkey = "a".repeat(64);

function observedJournal() {
  return buildMissionJournal(
    normalizeActivityEvents([
      {
        seq: 1,
        timestamp: "2026-08-21T14:00:00.000Z",
        kind: "turn_started",
        sourceEventId: sourceId,
        sourcePubkey: "agent-a",
        sourceKind: 24200,
        sourceCreatedAt: 1_787_319_600,
        sourceSignature: "agent-signature",
        origin: "historical_backfill",
        agentIndex: 0,
        channelId: "channel-1",
        sessionId: "session-1",
        turnId: "turn-1",
        payload: { triggeringEventIds: ["message-1"] },
      },
      {
        seq: 2,
        timestamp: "2026-08-21T14:01:00.000Z",
        kind: "turn_completed",
        sourceEventId: terminalSourceId,
        sourcePubkey: "agent-a",
        sourceKind: 24200,
        sourceCreatedAt: 1_787_319_660,
        sourceSignature: "agent-signature-2",
        origin: "historical_backfill",
        agentIndex: 0,
        channelId: "channel-1",
        sessionId: "session-1",
        turnId: "turn-1",
        payload: {},
      },
    ]),
  );
}

function artifact(overrides = {}) {
  const journal = observedJournal();
  return {
    ownerPubkey: "owner-a",
    relayUrl,
    agentPubkey,
    eventId: "c".repeat(64),
    signature: "owner-signature",
    createdAt: 1_787_319_720,
    artifactType: "verification",
    journalId: journal.id,
    correlationId: journal.correlationId,
    revision: 1,
    summary: null,
    note: null,
    receiptRef: "receipt:independent-check-1",
    sourceEventIds: [sourceId, terminalSourceId].sort(),
    ...overrides,
  };
}

function applyAuthority(
  journal,
  artifacts,
  scopedRelay = relayUrl,
  scopedAgent = agentPubkey,
) {
  return applyValidatedJournalAuthority(
    journal,
    artifacts,
    scopedRelay,
    scopedAgent,
  );
}

test("stable journal correlation preserves pre-day authority across midnight", () => {
  const currentSourceId = "d".repeat(64);
  const journal = buildMissionJournal(
    normalizeActivityEvents([
      {
        seq: 2,
        timestamp: "2026-08-22T00:00:05.000Z",
        kind: "acp_read",
        sourceEventId: currentSourceId,
        sourcePubkey: "agent-a",
        sourceKind: 24200,
        sourceCreatedAt: 1_787_356_805,
        sourceSignature: "agent-signature-current",
        origin: "historical_backfill",
        agentIndex: 0,
        channelId: "channel-1",
        sessionId: "session-1",
        turnId: "turn-1",
        payload: {
          method: "session/update",
          params: {
            update: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-after-midnight",
              status: "completed",
              output: { ok: true },
            },
          },
        },
      },
    ]),
  );
  assert.equal(journal.correlationId, journal.id);

  const overridden = applyAuthority(
    journal,
    [
      artifact({
        artifactType: "owner_override",
        journalId: journal.id,
        correlationId: journal.id,
        createdAt: 1_787_319_720,
        summary: "Owner summary written before midnight",
        receiptRef: null,
        sourceEventIds: [],
      }),
    ],
    relayUrl,
  );
  assert.equal(overridden.summary, "Owner summary written before midnight");

  const staleVerification = applyAuthority(
    journal,
    [
      artifact({
        journalId: journal.id,
        correlationId: journal.id,
        sourceEventIds: [sourceId],
      }),
    ],
    relayUrl,
  );
  assert.notEqual(staleVerification.proofState, "VERIFIED");

  const currentVerification = applyAuthority(
    journal,
    [
      artifact({
        journalId: journal.id,
        correlationId: journal.id,
        sourceEventIds: [sourceId, currentSourceId].sort(),
      }),
    ],
    relayUrl,
  );
  assert.equal(currentVerification.proofState, "VERIFIED");
});

test("owner-signed verification only promotes evidence bound to the journal", () => {
  const journal = observedJournal();
  assert.notEqual(journal.proofState, "VERIFIED");

  const verified = applyAuthority(journal, [artifact()], relayUrl);
  assert.equal(verified.proofState, "VERIFIED");
  assert.equal(verified.status, "completed");
  assert.equal(verified.events.at(-1).title, "Owner verification");
  assert.deepEqual(verified.events.at(-1).provenance.triggeringEventIds, [
    sourceId,
    terminalSourceId,
  ]);
  assert.equal(verified.events.at(-1).provenance.seq, 3);
  assert.equal(
    verified.events.at(-1).provenance.sourceSignature,
    "owner-signature",
  );

  const crossJournal = applyAuthority(
    journal,
    [artifact({ sourceEventIds: ["f".repeat(64)] })],
    relayUrl,
  );
  assert.notEqual(crossJournal.proofState, "VERIFIED");
});

test("later failure and incomplete states outrank an older verification", () => {
  const journal = observedJournal();

  for (const [status, proofState] of [
    ["failed", "FAILED"],
    ["incomplete", "UNKNOWN"],
  ]) {
    const terminalJournal = { ...journal, status, proofState };
    const updated = applyAuthority(terminalJournal, [artifact()], relayUrl);

    assert.equal(updated.status, status);
    assert.equal(updated.proofState, proofState);
    assert.equal(
      updated.events.some((event) => event.title === "Owner verification"),
      false,
    );
  }
});

test("later successful tool evidence invalidates an older verification", () => {
  const journal = observedJournal();
  const laterSourceId = "d".repeat(64);
  const laterTool = {
    ...journal.events[0],
    id: "later-tool",
    category: "tool",
    title: "Tool completed",
    status: "completed",
    proofState: "RECEIPTED",
    timestamp: "2026-08-21T14:03:00.000Z",
    provenance: {
      ...journal.events[0].provenance,
      sourceEventId: laterSourceId,
      sourceCreatedAt: 1_787_319_780,
      seq: 3,
      timestamp: "2026-08-21T14:03:00.000Z",
    },
  };
  const current = {
    ...journal,
    endedAt: laterTool.timestamp,
    eventCount: journal.eventCount + 1,
    events: [...journal.events, laterTool],
  };

  assert.notEqual(
    applyAuthority(current, [artifact()], relayUrl).proofState,
    "VERIFIED",
  );
  assert.equal(
    applyAuthority(
      current,
      [
        artifact({
          sourceEventIds: [sourceId, terminalSourceId, laterSourceId].sort(),
        }),
      ],
      relayUrl,
    ).proofState,
    "VERIFIED",
  );
});

test("later turn completion invalidates an older verification", () => {
  const journal = observedJournal();
  const laterSourceId = "e".repeat(64);
  const laterTurn = {
    ...journal.events.at(-1),
    id: "later-turn",
    timestamp: "2026-08-21T14:04:00.000Z",
    provenance: {
      ...journal.events.at(-1).provenance,
      sourceEventId: laterSourceId,
      sourceCreatedAt: 1_787_319_840,
      seq: 4,
      timestamp: "2026-08-21T14:04:00.000Z",
    },
  };
  const current = {
    ...journal,
    endedAt: laterTurn.timestamp,
    eventCount: journal.eventCount + 1,
    events: [...journal.events, laterTurn],
  };

  assert.notEqual(
    applyAuthority(current, [artifact()], relayUrl).proofState,
    "VERIFIED",
  );
});

test("every later journal frame invalidates an older verification", () => {
  const journal = observedJournal();
  for (const [index, category] of [
    "message",
    "prompt",
    "thought",
    "plan",
  ].entries()) {
    const laterSourceId = (index + 10).toString(16).padStart(64, "0");
    const laterEvent = {
      ...journal.events[0],
      id: `later-${category}`,
      category,
      title: `Later ${category}`,
      status: "completed",
      proofState: category === "prompt" ? "RECEIPTED" : "CLAIMED",
      timestamp: `2026-08-21T14:0${index + 3}:00.000Z`,
      provenance: {
        ...journal.events[0].provenance,
        sourceEventId: laterSourceId,
        sourceCreatedAt: 1_787_319_780 + index * 60,
        observerKind: "acp_read",
        seq: index + 3,
        timestamp: `2026-08-21T14:0${index + 3}:00.000Z`,
      },
    };
    const current = {
      ...journal,
      endedAt: laterEvent.timestamp,
      eventCount: journal.eventCount + 1,
      events: [...journal.events, laterEvent],
    };

    assert.notEqual(
      applyAuthority(current, [artifact()], relayUrl).proofState,
      "VERIFIED",
      category,
    );
    assert.equal(
      applyAuthority(
        current,
        [
          artifact({
            sourceEventIds: [sourceId, terminalSourceId, laterSourceId].sort(),
          }),
        ],
        relayUrl,
      ).proofState,
      "VERIFIED",
      category,
    );
  }
});

test("reapplying authority ignores its synthetic owner verification event", () => {
  const journal = observedJournal();
  const verified = applyAuthority(journal, [artifact()], relayUrl);
  const reapplied = applyAuthority(verified, [artifact()], relayUrl);

  assert.equal(reapplied.proofState, "VERIFIED");
  assert.equal(reapplied.events.at(-1).title, "Owner verification");
});

test("large journal verification computes its sequence without argument spread", () => {
  const journal = observedJournal();
  const template = journal.events.at(-1);
  const events = [
    journal.events[0],
    ...Array.from({ length: 149_999 }, (_, index) => ({
      ...template,
      id: `large-${index}`,
      provenance: {
        ...template.provenance,
        seq: index + 2,
      },
    })),
  ];
  const largeJournal = {
    ...journal,
    eventCount: events.length,
    events,
  };

  const verified = applyAuthority(largeJournal, [artifact()], relayUrl);
  assert.equal(verified.proofState, "VERIFIED");
  assert.equal(verified.events.at(-1).provenance.seq, 150_001);
});

test("verification writes use the journal correlation, not a tool-call correlation", () => {
  const journal = observedJournal();
  const receiptSourceId = "e".repeat(64);
  const toolEvidence = {
    ...journal.events[0],
    correlationId: "tool-call-1",
    proofState: "RECEIPTED",
    category: "tool",
    toolCallId: "tool-call-1",
    provenance: {
      ...journal.events[0].provenance,
      sourceEventId: receiptSourceId,
      toolCallId: "tool-call-1",
      triggeringEventIds: [],
    },
  };
  const journalWithTool = {
    ...journal,
    events: [...journal.events, toolEvidence],
  };

  assert.equal(journalAuthorityCorrelationId(journalWithTool), journal.id);
  assert.notEqual(
    journalAuthorityCorrelationId(journalWithTool),
    toolEvidence.correlationId,
  );
  assert.deepEqual(journalVerificationSources(journalWithTool), {
    sourceEventIds: [receiptSourceId],
    hasReceiptedEvidence: true,
    hasCorrelationEvidence: true,
    hasSupportedSourceSet: true,
    overflowCount: 0,
  });

  const missingCorrelationSource = {
    ...journalWithTool,
    correlationId: "message-without-an-observer-source",
  };
  assert.equal(
    journalVerificationSources(missingCorrelationSource).hasCorrelationEvidence,
    false,
  );
});

test("verification source capacity fails closed before backend submission", () => {
  const journal = observedJournal();
  const receipted = Array.from({ length: 257 }, (_, index) => ({
    ...journal.events[0],
    id: `receipt-${index}`,
    category: "tool",
    proofState: "RECEIPTED",
    provenance: {
      ...journal.events[0].provenance,
      sourceEventId: index.toString(16).padStart(64, "0"),
      triggeringEventIds: [],
    },
  }));
  const atLimit = journalVerificationSources({
    ...journal,
    events: [...journal.events, ...receipted.slice(0, 256)],
  });
  assert.equal(atLimit.sourceEventIds.length, 256);
  assert.equal(atLimit.hasSupportedSourceSet, true);
  assert.equal(atLimit.overflowCount, 0);

  const overLimit = journalVerificationSources({
    ...journal,
    events: [...journal.events, ...receipted.slice(0, 257)],
  });
  assert.equal(overLimit.sourceEventIds.length, 257);
  assert.equal(overLimit.hasSupportedSourceSet, false);
  assert.equal(overLimit.overflowCount, 1);
});

test("latest owner override changes summary without changing proof", () => {
  const journal = observedJournal();
  const base = artifact({
    artifactType: "owner_override",
    eventId: "d".repeat(64),
    summary: "First owner summary",
    note: "clarified",
    receiptRef: null,
    sourceEventIds: [],
  });
  const latest = {
    ...base,
    eventId: "e".repeat(64),
    revision: 2,
    createdAt: base.createdAt + 10,
    summary: "Corrected owner summary",
  };

  const updated = applyAuthority(journal, [latest, base], relayUrl);
  assert.equal(updated.summary, "Corrected owner summary");
  assert.equal(updated.summarySource, "owner");
  assert.equal(updated.ownerModifiedBy, "owner-a");
  assert.equal(updated.proofState, journal.proofState);

  const crossRelay = applyAuthority(
    journal,
    [{ ...latest, relayUrl: "wss://other-relay.example" }],
    relayUrl,
  );
  assert.equal(crossRelay.summarySource, "auto");
  assert.equal(crossRelay.summary, journal.summary);
});

test("same journal authority cannot cross managed-agent identity", () => {
  const journal = observedJournal();
  const crossAgent = applyAuthority(
    journal,
    [artifact({ agentPubkey: "b".repeat(64) })],
    relayUrl,
    agentPubkey,
  );
  assert.notEqual(crossAgent.proofState, "VERIFIED");
});
