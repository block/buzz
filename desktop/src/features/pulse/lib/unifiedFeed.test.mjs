import assert from "node:assert/strict";
import { test } from "node:test";
import { buildPulseConversations, matchesPulseFilter } from "./unifiedFeed.ts";
import { fetchPulseFeed } from "./fetchPulseFeed.ts";

const alice = "a".repeat(64);
const me = "b".repeat(64);
const agent = "c".repeat(64);
const channel = (id, channelType = "stream", visibility = "open") => ({
  id,
  name: id,
  channelType,
  visibility,
  isMember: true,
  archivedAt: null,
  participants: ["Alice", "You"],
  participantPubkeys: [alice, me],
});
const event = (id, channelId, tags = [], pubkey = alice, kind = 9) => ({
  id: id.repeat(64),
  kind,
  pubkey,
  content: `Message ${id}`,
  created_at: id.charCodeAt(0),
  sig: "",
  tags: [...(channelId ? [["h", channelId]] : []), ...tags],
});
const build = (
  events,
  channels = [channel("design"), channel("direct", "dm", "private")],
) => buildPulseConversations(events, channels, me, {}, new Set([agent]));

test("groups original threads, deduplicates events, and keeps channel/DM scope", () => {
  const root = event("a", "design");
  const reply = event("b", "design", [["e", root.id, "", "reply"]], agent);
  const dm = event("c", "direct", [["p", me]]);
  const items = build([
    root,
    reply,
    reply,
    dm,
    event("d", "inaccessible"),
    event("e", null),
  ]);
  assert.equal(items.length, 2);
  assert.equal(items.find((i) => i.channel.id === "design").messages.length, 2);
  const privateItem = items.find((i) => i.channel.id === "direct");
  assert.equal(privateItem.isPrivate, true);
  assert.equal(privateItem.isMention, true);
  assert.equal(
    matchesPulseFilter(privateItem, "dm", true, true, "message"),
    true,
  );
  assert.equal(
    matchesPulseFilter(privateItem, "channel", false, false, ""),
    false,
  );
  assert.equal(
    matchesPulseFilter(
      items.find((i) => i.channel.id === "design"),
      "agent",
      false,
      false,
      "",
    ),
    true,
  );
});

test("private and agent filters intersect; removal of membership removes content", () => {
  const message = event("a", "private", [], agent);
  const privateChannel = channel("private", "stream", "private");
  const [item] = build([message], [privateChannel]);
  assert.equal(matchesPulseFilter(item, "agent", true, false, ""), true);
  assert.deepEqual(
    build([message], [{ ...privateChannel, isMember: false }]),
    [],
  );
  assert.deepEqual(
    build([message], [{ ...privateChannel, archivedAt: "today" }]),
    [],
  );
});

test("public notes stay channel-less and project comments are excluded", () => {
  const note = event("a", null, [], alice, 1);
  const projectComment = event(
    "b",
    null,
    [["a", "30617:owner:repo"]],
    alice,
    1,
  );
  const [item] = build([note, projectComment]);
  assert.equal(item.channel, null);
  assert.equal(item.isPrivate, false);
  assert.equal(matchesPulseFilter(item, "note", false, false, ""), true);
});

test("production formatter applies edits, deletions, and retracted edits", () => {
  const root = event("a", "design");
  const edit = {
    ...event("b", "design", [["e", root.id]], alice, 40003),
    content: "Corrected message",
  };
  assert.equal(build([root, edit])[0].messages[0].body, "Corrected message");
  const retract = event("c", "design", [["e", edit.id]], alice, 5);
  assert.equal(build([root, edit, retract])[0].messages[0].body, root.content);
  assert.deepEqual(
    build([root, event("d", "design", [["e", root.id]], alice, 5)]),
    [],
  );
});

test("fetch gives DMs a separate quota, loads roots and structural closure", async () => {
  const root = event("a", "design");
  const reply = event("b", "design", [["e", root.id, "", "reply"]]);
  const edit = event("c", "design", [["e", reply.id]], alice, 40003);
  const calls = [];
  const result = await fetchPulseFeed(
    ["direct", "design"],
    [me],
    async (filter) => {
      calls.push(filter);
      if (filter.ids) return [root, event("z", "outsider")];
      if (filter.kinds.includes(40003)) return [edit];
      if (filter.kinds.includes(9))
        return filter["#h"].includes("design") ? [reply] : [];
      return [];
    },
    ["direct"],
  );
  assert.deepEqual(calls[0]["#h"], ["direct"]);
  assert.deepEqual(calls[1]["#h"], ["design"]);
  assert.ok(result.some((e) => e.id === root.id));
  assert.ok(!result.some((e) => e.id === "z".repeat(64)));
  assert.ok(
    calls.some((f) => f.kinds.includes(5) && f["#e"]?.includes(edit.id)),
  );
  assert.ok(calls.every((f) => f.kinds.length > 0 && f.limit <= 300));
});

test("source and structural read failures propagate for visible retry", async () => {
  await assert.rejects(
    fetchPulseFeed(["design"], [], async () => {
      throw new Error("offline");
    }),
    /offline/,
  );
  await assert.rejects(
    fetchPulseFeed(["design"], [], async (filter) => {
      if (filter.kinds.includes(9)) return [event("a", "design")];
      throw new Error("edits unavailable");
    }),
    /edits unavailable/,
  );
  await assert.rejects(
    fetchPulseFeed(
      Array.from({ length: 129 }, (_, i) => `${i}`),
      [],
      async () => [],
    ),
    /Too many/,
  );
});
