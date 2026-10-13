import assert from "node:assert/strict";
import test from "node:test";
import { buildNotificationInboxItems, participatedRoots } from "./notificationInbox.ts";

const me = "a".repeat(64), other = "b".repeat(64);
const ch = [{ id: "dm1", name: "Bob", channelType: "dm" }, { id: "c1", name: "gen", channelType: "stream" }];
const ev = (id, pubkey, tags, t = 1) => ({ id, kind: 9, pubkey, content: "x", created_at: t, tags, sig: "" });
const base = { me, channels: ch, dmEnabled: true, threadRepliesEnabled: true, mutedChannelIds: new Set(), mutedRootIds: new Set() };

test("DMs and participated thread replies become inbox rows; mentions and own skipped", () => {
  const dms = [ev("d1", other, [["h", "dm1"]]), ev("d2", me, [["h", "dm1"]]), ev("d3", other, [["h", "dm1"], ["p", me]])];
  const mine = [ev("r1", me, [["h", "c1"]])];
  const roots = participatedRoots(mine, []);
  const replies = [ev("x1", other, [["h", "c1"], ["e", "r1", "", "root"], ["e", "r1", "", "reply"]]), ev("x2", other, [["h", "c1"], ["e", "zz", "", "root"], ["e", "zz", "", "reply"]])];
  const ids = buildNotificationInboxItems(dms, replies, roots, base).map((i) => i.id).sort();
  assert.deepEqual(ids, ["d1", "x1"]);
});

test("switches and mutes silence rows", () => {
  const dms = [ev("d1", other, [["h", "dm1"]])];
  assert.equal(buildNotificationInboxItems(dms, [], new Set(), { ...base, dmEnabled: false }).length, 0);
  assert.equal(buildNotificationInboxItems(dms, [], new Set(), { ...base, mutedChannelIds: new Set(["dm1"]) }).length, 0);
  const replies = [ev("x1", other, [["h", "c1"], ["e", "r1", "", "root"], ["e", "r1", "", "reply"]])];
  assert.equal(buildNotificationInboxItems([], replies, new Set(["r1"]), { ...base, mutedRootIds: new Set(["r1"]) }).length, 0);
});
