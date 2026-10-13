import assert from "node:assert/strict";
import test from "node:test";
import { _catchUpForTests, observedUnreadHandlers as h } from "./observedUnread.ts";

const ME = "a".repeat(64), BOB = "b".repeat(64);
const scope = { pubkey: ME, relayUrl: "wss://r" };

test("observed unread: ingest projects counts, markers clear, thread markers clear only thread events", () => {
  const snap = h.observed_unread_open_scope({ request: { scope, legacyPayload: null, membershipSeed: { participatedRootIds: ["root1"], authoredRootIds: [], mentionedRootIds: [], followedRootIds: [], mutedRootIds: [], mutedChannelIds: [] } } });
  assert.equal(snap.kind, "snapshot");
  const ev = (id, createdAt, rootId) => ({ channelId: "c1", id, createdAt, rootId, highPriority: !!rootId, countsTowardBadge: !!rootId, countsTowardAppBadge: false });
  let d = h.observed_unread_ingest({ request: { scope, sequence: 1, baseRevision: snap.revision, events: [ev("m1", 10, null), ev("t1", 11, "root1")], channelLatest: [], markers: [], membership: [], clearChannels: [], clearAll: false } });
  const p = d.upserts.find((x) => x.channelId === "c1");
  assert.equal(p.count, 2);
  assert.equal(p.badgeCount, 1);
  assert.equal(p.topLevelUnread, true);
  d = h.observed_unread_ingest({ request: { scope, sequence: 2, baseRevision: d.revision, events: [], channelLatest: [], markers: [{ contextId: "thread:root1", readAt: 11 }], membership: [], clearChannels: [], clearAll: false } });
  assert.equal(d.upserts[0].count, 1);
  d = h.observed_unread_ingest({ request: { scope, sequence: 3, baseRevision: d.revision, events: [], channelLatest: [], markers: [{ contextId: "c1", readAt: 20 }], membership: [], clearChannels: [], clearAll: false } });
  assert.deepEqual(d.removed, ["c1"]);
});

test("unread_catch_up: top-level + participated thread replies notify, unrelated thread replies don't", async () => {
  const tags = (extra) => [["h", "c2"], ...extra];
  const events = [
    { id: "x1", kind: 9, pubkey: BOB, content: "", created_at: 50, tags: tags([]) },
    { id: "x2", kind: 9, pubkey: BOB, content: "", created_at: 51, tags: tags([["e", "root1", "", "root"], ["e", "root1", "", "reply"]]) },
    { id: "x3", kind: 9, pubkey: BOB, content: "", created_at: 52, tags: tags([["e", "other", "", "reply"]]) },
  ];
  const r = await _catchUpForTests({ request: { channels: [{ id: "c2", type: "stream", name: "c2", readAt: 40 }], selfPubkey: ME, mutedChannelIds: [] } }, async (f) => {
    assert.equal(f.since, 41);
    return events;
  });
  const c = r.channels[0];
  assert.equal(c.status, "success");
  assert.deepEqual(c.observedEvents.map((e) => e.id), ["x1", "x2"]);
  assert.deepEqual(c.activityRows.map((e) => e.id), ["x2"]);
});
