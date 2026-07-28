import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCalendarEvent,
  buildRevisionEvents,
  buildSourceEvent,
  parseRelayCalendarEvent,
  parseRelaySourceEvent,
  revisionManifestHash,
  setBattleRhythmEventSignerForTests,
} from "./eventCodec.ts";

const event = {
  schemaVersion: 1,
  id: "event-1",
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

test("calendar event uses stable d and parsed temporal/source tags", async () => {
  const relay = await buildCalendarEvent(event, 10);
  assert.deepEqual(relay.tags, [
    ["d", "event-1"],
    ["start", event.start],
    ["end", event.end],
    ["source", "fas"],
    ["revision", "r1"],
  ]);
  assert.ok(relay.created_at > 10);
  assert.equal(parseRelayCalendarEvent(relay)?.id, event.id);
});
test("source uses stable d and coverage tags", async () => {
  const source = {
    schemaVersion: 1,
    id: "fas",
    type: "fas",
    displayName: "FAS",
    coverageStart: "2026-08-01T00:00:00+10:00",
    coverageEnd: "2026-08-31T00:00:00+10:00",
    documentName: "fas.pdf",
    documentHash: "a".repeat(64),
    revisionId: "r1",
    priorRevisionId: null,
    importedAt: "2026-07-28T10:00:00+10:00",
    status: "approved",
    sourceReference: "trusted://fas",
  };
  const relay = await buildSourceEvent(source);
  assert.deepEqual(relay.tags, [
    ["d", "fas"],
    ["source", "fas"],
    ["revision", "r1"],
    ["start", source.coverageStart],
    ["end", source.coverageEnd],
  ]);
  assert.equal(
    parseRelaySourceEvent({
      ...relay,
      tags: relay.tags.filter((tag) => tag[0] !== "end"),
    }),
    null,
  );
});
test("relay decoder rejects malformed source ownership tags", async () => {
  const relay = await buildCalendarEvent(event);
  relay.tags = relay.tags.filter((tag) => tag[0] !== "revision");
  assert.equal(parseRelayCalendarEvent(relay), null);
});
test("revision splits ordered immutable chunks below 240 KiB with common manifest", async () => {
  const revision = {
    schemaVersion: 1,
    id: "r1",
    sourceId: "fas",
    priorRevisionId: null,
    importedAt: "2026-07-28T10:00:00+10:00",
    changes: Array.from({ length: 80 }, (_, i) => ({
      kind: "added",
      after: { ...event, id: `event-${i}`, description: "x".repeat(4000) },
    })),
  };
  const chunks = await buildRevisionEvents(revision);
  assert.ok(chunks.length > 1);
  const hashes = new Set(
    chunks.map((chunk) => chunk.tags.find((t) => t[0] === "hash")?.[1]),
  );
  assert.equal(hashes.size, 1);
  const firstChunk = JSON.parse(chunks[0].content);
  assert.equal(
    firstChunk.manifestHash,
    await revisionManifestHash({ ...revision, changes: revision.changes }),
  );
  const changed = { ...event, title: "Updated Sail" };
  const beforeAfter = await buildRevisionEvents({
    schemaVersion: 1,
    id: "r2",
    sourceId: "fas",
    priorRevisionId: "r1",
    importedAt: "2026-07-28T10:00:00+10:00",
    changes: [
      { kind: "changed", before: event, after: changed },
      { kind: "removed", before: event },
    ],
  });
  const decoded = JSON.parse(beforeAfter[0].content);
  assert.equal(decoded.changes[0].before.title, "Sail");
  assert.equal(decoded.changes[0].after.title, "Updated Sail");
  assert.equal(decoded.changes[1].before.id, "event-1");
  chunks.forEach((chunk, index) => {
    assert.ok(new TextEncoder().encode(chunk.content).byteLength <= 240 * 1024);
    assert.deepEqual(
      chunk.tags.find((t) => t[0] === "chunk"),
      ["chunk", String(index), String(chunks.length)],
    );
  });
});
