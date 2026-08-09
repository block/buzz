import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  parseBattleRhythmEvent,
  parseBattleRhythmRevision,
  parseBattleRhythmSource,
} from "./contracts.ts";

const source = {
  schemaVersion: 1,
  id: "fas-2026",
  type: "fas",
  displayName: "FAS 2026",
  coverageStart: "2026-08-01T00:00:00+10:00",
  coverageEnd: "2026-08-31T00:00:00+10:00",
  documentName: "fas.pdf",
  documentHash: "a".repeat(64),
  revisionId: "rev-1",
  priorRevisionId: null,
  importedAt: "2026-07-28T10:00:00+10:00",
  status: "approved",
  sourceReference: "trusted-lan://fas.pdf",
};
const event = {
  schemaVersion: 1,
  id: "event-1",
  ownership: { kind: "manual" },
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
  recurrence: {
    frequency: "weekly",
    interval: 1,
    until: "2026-08-31T08:00:00+10:00",
    seriesId: "sail-routine",
  },
  excludedOccurrenceStarts: ["2026-08-17T08:00:00+10:00"],
};
const fixture = JSON.parse(
  readFileSync(
    new URL("./fixtures/contracts-v1.json", import.meta.url),
    "utf8",
  ),
);

test("parsers accept exact immutable calendar contracts", () => {
  const parsed = parseBattleRhythmEvent(event);
  assert.equal(parsed.title, "Sail");
  assert.ok(Object.isFrozen(parsed));
  assert.ok(Object.isFrozen(parsed.participants));
  assert.equal(parseBattleRhythmSource(source).id, "fas-2026");
});
test("TypeScript consumes the shared v1 fixture", () => {
  assert.equal(parseBattleRhythmSource(fixture.source).id, "fas-2026");
  assert.equal(parseBattleRhythmEvent(fixture.event).id, "event-1");
});

test("manual event parser rejects source revision ownership", () => {
  assert.throws(() =>
    parseBattleRhythmEvent({
      ...event,
      ownership: { kind: "manual", sourceId: "fas" },
    }),
  );
  assert.throws(() =>
    parseBattleRhythmEvent({
      ...event,
      recurrence: { ...event.recurrence, until: null },
      excludedOccurrenceStarts: ["2026-08-10T08:00:00Z"],
    }),
  );
  assert.equal(
    parseBattleRhythmEvent({
      ...event,
      start: "2026-09-28T08:00:00+10:00",
      end: "2026-09-28T09:00:00+10:00",
      recurrence: {
        frequency: "weekly",
        interval: 1,
        until: "2026-10-12T08:00:00+11:00",
        seriesId: "dst-routine",
      },
      excludedOccurrenceStarts: ["2026-10-05T08:00:00+11:00"],
    }).excludedOccurrenceStarts[0],
    "2026-10-05T08:00:00+11:00",
  );
});

test("parsers reject unknown fields, non-ISO dates, unordered coverage, and invalid all-day values", () => {
  assert.throws(() => parseBattleRhythmEvent({ ...event, extra: true }));
  assert.throws(() => parseBattleRhythmEvent({ ...event, start: "tomorrow" }));
  assert.throws(() =>
    parseBattleRhythmEvent({ ...event, start: "2026-02-30T08:00:00+10:00" }),
  );
  assert.throws(() => parseBattleRhythmEvent({ ...event, start: event.end }));
  assert.throws(() => parseBattleRhythmEvent({ ...event, allDay: "false" }));
  assert.throws(() =>
    parseBattleRhythmSource({ ...source, coverageStart: source.coverageEnd }),
  );
});

test("recurrence is strict, bounded, and only excludes occurrences in its series", () => {
  for (const frequency of ["daily", "weekly", "monthly"]) {
    const parsed = parseBattleRhythmEvent({
      ...event,
      recurrence: { ...event.recurrence, frequency },
      excludedOccurrenceStarts: [],
    });
    assert.equal(parsed.recurrence.frequency, frequency);
  }
  assert.throws(() =>
    parseBattleRhythmEvent({
      ...event,
      recurrence: { ...event.recurrence, interval: 0 },
    }),
  );
  assert.throws(() =>
    parseBattleRhythmEvent({
      ...event,
      excludedOccurrenceStarts: [
        event.excludedOccurrenceStarts[0],
        event.excludedOccurrenceStarts[0],
      ],
    }),
  );
  assert.throws(() =>
    parseBattleRhythmEvent({
      ...event,
      excludedOccurrenceStarts: ["2026-08-18T08:00:00+10:00"],
    }),
  );
});

test("revision bounds proposed entries and preserves before after payloads", () => {
  const revision = parseBattleRhythmRevision({
    schemaVersion: 1,
    id: "rev-1",
    sourceId: "fas-2026",
    priorRevisionId: null,
    importedAt: "2026-07-28T10:00:00+10:00",
    changes: [{ kind: "added", after: event }],
  });
  assert.equal(revision.changes[0].kind, "added");
  assert.throws(() =>
    parseBattleRhythmRevision({
      ...revision,
      changes: Array.from({ length: 2001 }, () => ({
        kind: "added",
        after: event,
      })),
    }),
  );
});
