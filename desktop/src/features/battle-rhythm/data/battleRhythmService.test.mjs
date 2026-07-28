import assert from "node:assert/strict";
import test from "node:test";
import { applyImportRevision } from "./battleRhythmService.ts";
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
test("import publishes heads then chunks then active source pointer", async () => {
  const kinds = [];
  await applyImportRevision(
    {
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
        publishEvent: async (e) => {
          calls++;
          return e;
        },
      },
    ),
  );
  assert.equal(calls, 0);
});
