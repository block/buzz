import assert from "node:assert/strict";
import test from "node:test";

import {
  activityLedgerDayRange,
  applyAuthorityToTodayActivity,
  buildTodayActivityFromArchivedEvents,
} from "./activityLedgerToday.ts";

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
});

test("day range is half-open and rejects impossible dates", () => {
  const range = activityLedgerDayRange("2026-08-21");
  assert.equal(range.endCreatedAt - range.startCreatedAt, 24 * 60 * 60);
  assert.throws(() => activityLedgerDayRange("2026-02-30"));
  assert.throws(() => activityLedgerDayRange("08/21/2026"));
});

test("Today authority overlay recomputes evidence-gap counts", async () => {
  const event = relayEvent({
    id: "a".repeat(64),
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
    decoded: { ...event.decoded, seq: 2, kind: "turn_completed" },
  });
  const surface = await buildTodayActivityFromArchivedEvents({
    day: "2026-08-21",
    agents: [{ pubkey: "agent-a", name: "Honey" }],
    events: [event, ended],
    decrypt: async (candidate) => candidate.decoded,
  });
  const journal = surface.journals[0];
  const updated = applyAuthorityToTodayActivity(surface, [
    {
      ownerPubkey: "owner-a",
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
      sourceEventIds: [event.id],
    },
  ]);

  assert.equal(updated.journals[0].proofState, "VERIFIED");
  assert.equal(updated.counts.claimedWithoutEvidence, 0);
  assert.equal(updated.channels[0].lastActivityAt, "2026-08-21T14:00:01.000Z");
});
