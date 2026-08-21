import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMissionJournal,
  normalizeActivityEvents,
} from "./activityLedger.ts";
import { applyValidatedJournalAuthority } from "./activityLedgerAuthority.ts";

const sourceId = "a".repeat(64);

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
        sourceEventId: "b".repeat(64),
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
    sourceEventIds: [sourceId],
    ...overrides,
  };
}

test("owner-signed verification only promotes evidence bound to the journal", () => {
  const journal = observedJournal();
  assert.notEqual(journal.proofState, "VERIFIED");

  const verified = applyValidatedJournalAuthority(journal, [artifact()]);
  assert.equal(verified.proofState, "VERIFIED");
  assert.equal(verified.status, "completed");
  assert.equal(verified.events.at(-1).title, "Owner verification");
  assert.deepEqual(verified.events.at(-1).provenance.triggeringEventIds, [
    sourceId,
  ]);
  assert.equal(
    verified.events.at(-1).provenance.sourceSignature,
    "owner-signature",
  );

  const crossJournal = applyValidatedJournalAuthority(journal, [
    artifact({ sourceEventIds: ["f".repeat(64)] }),
  ]);
  assert.notEqual(crossJournal.proofState, "VERIFIED");
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

  const updated = applyValidatedJournalAuthority(journal, [latest, base]);
  assert.equal(updated.summary, "Corrected owner summary");
  assert.equal(updated.summarySource, "owner");
  assert.equal(updated.ownerModifiedBy, "owner-a");
  assert.equal(updated.proofState, journal.proofState);
});
