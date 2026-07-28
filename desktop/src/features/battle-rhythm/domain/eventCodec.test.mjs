import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCalendarEvent,
  buildRevisionEvents,
  parseRelayCalendarEvent,
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
  chunks.forEach((chunk, index) => {
    assert.ok(new TextEncoder().encode(chunk.content).byteLength <= 240 * 1024);
    assert.deepEqual(
      chunk.tags.find((t) => t[0] === "chunk"),
      ["chunk", String(index), String(chunks.length)],
    );
  });
});
