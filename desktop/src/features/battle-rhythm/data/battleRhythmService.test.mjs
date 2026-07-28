import assert from "node:assert/strict";
import test from "node:test";
import {
  applyImportRevision,
  fetchBattleRhythm,
} from "./battleRhythmService.ts";
import { setBattleRhythmEventSignerForTests } from "../domain/eventCodec.ts";

const source = {
  schemaVersion: 1,
  id: "fas",
  type: "fas",
  displayName: "FAS",
  coverageStart: "2026-08-01T00:00:00+10:00",
  coverageEnd: "2026-08-31T00:00:00+10:00",
  documentName: "a",
  documentHash: "a".repeat(64),
  revisionId: "r1",
  priorRevisionId: null,
  importedAt: "2026-07-28T10:00:00+10:00",
  status: "approved",
  sourceReference: "trusted://a",
};
const event = {
  schemaVersion: 1,
  id: "e1",
  ownership: {
    kind: "source",
    sourceId: "fas",
    revisionId: "r1",
    sourceLocation: "p1",
  },
  title: "Sail",
  description: null,
  type: "passage",
  start: "2026-08-03T08:00:00+10:00",
  end: "2026-08-03T09:00:00+10:00",
  allDay: false,
  timeZone: "Australia/Sydney",
  status: "approved",
  location: null,
  responsibleOwner: null,
  participants: [],
  remarks: null,
  linkedPlanId: null,
  linkedTaskId: null,
  linkedMissionRequirementId: null,
  parentActivityId: null,
  recurrence: null,
  excludedOccurrenceStarts: [],
};
setBattleRhythmEventSignerForTests(async (input) => ({
  id: "test",
  pubkey: "owner",
  created_at: input.createdAt ?? 1,
  kind: input.kind,
  tags: input.tags,
  content: input.content,
  sig: "sig",
}));
const noExistingHeads = async () => [];
test("import publishes heads then chunks then active source pointer", async () => {
  const kinds = [];
  await applyImportRevision(
    {
      ownerPubkey: "owner",
      source,
      revision: {
        schemaVersion: 1,
        id: "r1",
        sourceId: "fas",
        priorRevisionId: null,
        importedAt: source.importedAt,
        changes: [{ kind: "added", after: event }],
      },
      events: [event],
    },
    {
      fetchEvents: noExistingHeads,
      publishEvent: async (e) => {
        kinds.push(e.kind);
        return e;
      },
    },
  );
  assert.deepEqual(kinds, [30631, 46310, 30630]);
});
test("import rejects an event owned by another source before any publish", async () => {
  let calls = 0;
  await assert.rejects(() =>
    applyImportRevision(
      {
        ownerPubkey: "owner",
        source,
        revision: {
          schemaVersion: 1,
          id: "r1",
          sourceId: "fas",
          priorRevisionId: null,
          importedAt: source.importedAt,
          changes: [],
        },
        events: [
          {
            ...event,
            ownership: {
              kind: "source",
              sourceId: "other",
              revisionId: "r1",
              sourceLocation: "p",
            },
          },
        ],
      },
      {
        fetchEvents: noExistingHeads,
        publishEvent: async (e) => {
          calls++;
          return e;
        },
      },
    ),
  );
  assert.equal(calls, 0);
});
test("import never replaces an existing manual head", async () => {
  let publishes = 0;
  const manual = { ...event, ownership: { kind: "manual" } };
  const manualHead = await (
    await import("../domain/eventCodec.ts")
  ).buildCalendarEvent(manual);
  await assert.rejects(() =>
    applyImportRevision(
      {
        ownerPubkey: "owner",
        source,
        revision: {
          schemaVersion: 1,
          id: "r1",
          sourceId: "fas",
          priorRevisionId: null,
          importedAt: source.importedAt,
          changes: [{ kind: "added", after: event }],
        },
        events: [event],
      },
      {
        fetchEvents: async () => [manualHead],
        publishEvent: async (relay) => {
          publishes++;
          return relay;
        },
      },
    ),
  );
  assert.equal(publishes, 0);
});
test("import never replaces an existing head from another source", async () => {
  let publishes = 0;
  const codec = await import("../domain/eventCodec.ts");
  const otherHead = await codec.buildCalendarEvent({
    ...event,
    ownership: {
      kind: "source",
      sourceId: "other",
      revisionId: "r9",
      sourceLocation: "p9",
    },
  });
  await assert.rejects(() =>
    applyImportRevision(
      {
        ownerPubkey: "owner",
        source,
        revision: {
          schemaVersion: 1,
          id: "r1",
          sourceId: "fas",
          priorRevisionId: null,
          importedAt: source.importedAt,
          changes: [{ kind: "added", after: event }],
        },
        events: [event],
      },
      {
        fetchEvents: async () => [otherHead],
        publishEvent: async (relay) => {
          publishes++;
          return relay;
        },
      },
    ),
  );
  assert.equal(publishes, 0);
});
test("fetch omits a source whose active revision has missing or hash-invalid chunks", async () => {
  const codec = await import("../domain/eventCodec.ts");
  const sourceHead = await codec.buildSourceEvent(source);
  const chunks = await codec.buildRevisionEvents({
    schemaVersion: 1,
    id: "r1",
    sourceId: "fas",
    priorRevisionId: null,
    importedAt: source.importedAt,
    changes: [{ kind: "added", after: event }],
  });
  const responses = [
    [sourceHead],
    [await codec.buildCalendarEvent(event)],
    chunks,
  ];
  const client = { fetchEvents: async () => responses.shift() ?? [] };
  assert.equal(
    (
      await fetchBattleRhythm(
        "owner",
        {
          start: "2026-08-01T00:00:00+10:00",
          end: "2026-09-01T00:00:00+10:00",
        },
        client,
      )
    ).sources.length,
    1,
  );
  const bad = {
    ...chunks[0],
    content: chunks[0].content.replace("event-1", "altered"),
  };
  const invalidClient = {
    fetchEvents: async () =>
      [[sourceHead], [await codec.buildCalendarEvent(event)], [bad]].shift() ??
      [],
  };
  assert.equal(
    (
      await fetchBattleRhythm(
        "owner",
        {
          start: "2026-08-01T00:00:00+10:00",
          end: "2026-09-01T00:00:00+10:00",
        },
        invalidClient,
      )
    ).sources.length,
    0,
  );
  const missingClient = {
    fetchEvents: async () =>
      [[sourceHead], [await codec.buildCalendarEvent(event)], []].shift() ?? [],
  };
  assert.equal(
    (
      await fetchBattleRhythm(
        "owner",
        {
          start: "2026-08-01T00:00:00+10:00",
          end: "2026-09-01T00:00:00+10:00",
        },
        missingClient,
      )
    ).sources.length,
    0,
  );
  const duplicateClient = {
    fetchEvents: async () =>
      [
        [sourceHead],
        [await codec.buildCalendarEvent(event)],
        [chunks[0], { ...chunks[0], id: "duplicate" }],
      ].shift() ?? [],
  };
  assert.equal(
    (
      await fetchBattleRhythm(
        "owner",
        {
          start: "2026-08-01T00:00:00+10:00",
          end: "2026-09-01T00:00:00+10:00",
        },
        duplicateClient,
      )
    ).sources.length,
    0,
  );
});
test("chunk publication failure leaves the active source pointer unwritten", async () => {
  const kinds = [];
  await assert.rejects(() =>
    applyImportRevision(
      {
        ownerPubkey: "owner",
        source,
        revision: {
          schemaVersion: 1,
          id: "r1",
          sourceId: "fas",
          priorRevisionId: null,
          importedAt: source.importedAt,
          changes: [{ kind: "added", after: event }],
        },
        events: [event],
      },
      {
        fetchEvents: noExistingHeads,
        publishEvent: async (relay) => {
          kinds.push(relay.kind);
          if (relay.kind === 46310) throw new Error("chunk failure");
          return relay;
        },
      },
    ),
  );
  assert.deepEqual(kinds, [30631, 46310]);
});
