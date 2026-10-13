import assert from "node:assert/strict";
import test from "node:test";
import { APPROVAL_KINDS, MENTION_KINDS, getFeed } from "./feed.ts";

const ME = "a".repeat(64), BOB = "b".repeat(64);
const ev = (id, kind, extra = {}) => ({ id, kind, pubkey: BOB, content: id, created_at: 100, tags: [["p", ME], ["h", "chan-1"]], ...extra });

test("get_feed: 2 batched queries, desktop filters, shape and edit suppression", async () => {
  const calls = [];
  const query = async (filters) => {
    calls.push(filters);
    if (filters[0].kinds[0] === 40003)
      return [
        { id: "e1", kind: 40003, pubkey: BOB, content: "", created_at: 101, tags: [["e", "m1"], ["link-preview", "none"]] },
        { id: "e2", kind: 40003, pubkey: "c".repeat(64), content: "", created_at: 101, tags: [["e", "m2"], ["link-preview", "none"]] },
      ];
    return [ev("m1", 9), ev("m2", 1618), ev("ap1", 46010)];
  };
  const out = await getFeed({ limit: 500, since: 50 }, ME, query);
  assert.equal(calls.length, 2);
  assert.ok(calls.every((c) => c.length <= 10));
  assert.deepEqual(calls[0][0], { kinds: MENTION_KINDS, "#p": [ME], limit: 100, since: 50 });
  assert.deepEqual(calls[0][1], { kinds: APPROVAL_KINDS, "#p": [ME], limit: 20, since: 50 });
  assert.deepEqual(calls[1][0], { kinds: [40003], "#e": ["m1", "m2"] });
  assert.deepEqual(out.feed.mentions.map((m) => m.id), ["m1", "m2"]);
  assert.equal(out.feed.mentions[0].category, "mention");
  assert.equal(out.feed.mentions[0].channel_id, "chan-1");
  assert.ok(out.feed.mentions[0].tags.some((t) => t[0] === "link-preview"), "author's edit suppresses preview");
  assert.ok(!out.feed.mentions[1].tags.some((t) => t[0] === "link-preview"), "non-author edit ignored");
  assert.deepEqual(out.feed.needs_action.map((m) => [m.id, m.category]), [["ap1", "needs_action"]]);
  assert.deepEqual(out.feed.activity, []);
  assert.deepEqual(out.feed.agent_activity, []);
  assert.equal(out.meta.total, 3);
  assert.equal(out.meta.since, 50);
});

test("get_feed: types filter and default limit; no edit query without mentions", async () => {
  const calls = [];
  const out = await getFeed({ types: "needs_action" }, ME, async (f) => (calls.push(f), []));
  assert.equal(calls.length, 1);
  assert.equal(calls[0].length, 1);
  assert.deepEqual(calls[0][0].kinds, APPROVAL_KINDS);
  assert.equal(out.meta.total, 0);
  const calls2 = [];
  await getFeed(undefined, ME, async (f) => (calls2.push(f), []));
  assert.equal(calls2[0][0].limit, 50);
});

test("get_feed: until pages backwards (exclusive) on both filters", async () => {
  const calls = [];
  await getFeed({ until: 500, limit: 50 }, ME, async (f) => (calls.push(f), []));
  assert.equal(calls[0][0].until, 499);
  assert.equal(calls[0][1].until, 499);
});
