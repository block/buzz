import assert from "node:assert/strict";
import test from "node:test";

import {
  applyReactionToggle,
  collectReactions,
  groupReactions,
  splitAgentStatus,
} from "./messageReactions.ts";

const TARGET = "a".repeat(64);
const OTHER_TARGET = "b".repeat(64);

let nextId = 0;
function reaction(content, { pubkey = "alice", tags = [] } = {}) {
  nextId += 1;
  return {
    id: `reaction-${nextId}`,
    kind: 7,
    pubkey,
    content,
    created_at: 1000 + nextId,
    tags: [["e", TARGET], ...tags],
  };
}

function deletion(targetId, kind = 5) {
  nextId += 1;
  return {
    id: `deletion-${nextId}`,
    kind,
    pubkey: "alice",
    content: "",
    created_at: 2000 + nextId,
    tags: [["e", targetId]],
  };
}

test("aggregates reactions by target message", () => {
  const events = [
    reaction("🔥"),
    reaction("🔥", { pubkey: "bob" }),
    { ...reaction("👀"), tags: [["e", OTHER_TARGET]] },
  ];
  const byTarget = collectReactions(events);
  assert.equal(byTarget.get(TARGET)?.length, 2);
  assert.equal(byTarget.get(OTHER_TARGET)?.length, 1);
});

test("empty and '+' content render as 👍", () => {
  const byTarget = collectReactions([
    reaction(""),
    reaction("+", { pubkey: "bob" }),
  ]);
  assert.deepEqual(
    byTarget.get(TARGET)?.map((entry) => entry.emoji),
    ["👍", "👍"],
  );
});

test("custom-emoji reactions carry the NIP-30 tag URL", () => {
  const byTarget = collectReactions([
    reaction(":eyes-intensifies:", {
      tags: [["emoji", "eyes-intensifies", "https://relay/media/eyes.png"]],
    }),
  ]);
  assert.deepEqual(byTarget.get(TARGET), [
    {
      emoji: ":eyes-intensifies:",
      emojiUrl: "https://relay/media/eyes.png",
      pubkey: "alice",
    },
  ]);
});

test("a shortcode without a matching emoji tag has no URL", () => {
  const byTarget = collectReactions([
    reaction(":mystery:", { tags: [["emoji", "other", "https://x/y.png"]] }),
  ]);
  assert.equal(byTarget.get(TARGET)?.[0]?.emojiUrl, undefined);
});

test("deleted reactions are dropped (kind 5 and 9005)", () => {
  const kept = reaction("🔥");
  const removedByFive = reaction("👀", { pubkey: "bob" });
  const removedByNine = reaction("🎉", { pubkey: "carol" });
  const byTarget = collectReactions([
    kept,
    removedByFive,
    removedByNine,
    deletion(removedByFive.id, 5),
    deletion(removedByNine.id, 9005),
  ]);
  assert.deepEqual(
    byTarget.get(TARGET)?.map((entry) => entry.emoji),
    ["🔥"],
  );
});

test("duplicate deliveries of the same reaction collapse to one", () => {
  const original = reaction("🔥");
  const byTarget = collectReactions([
    original,
    { ...original, id: "redelivered" },
  ]);
  assert.equal(byTarget.get(TARGET)?.length, 1);
});

test("groupReactions buckets by emoji in first-reacted order", () => {
  const groups = groupReactions([
    { emoji: "🔥", pubkey: "alice" },
    { emoji: "👀", pubkey: "bob" },
    { emoji: "🔥", pubkey: "carol" },
  ]);
  assert.deepEqual(groups, [
    { emoji: "🔥", emojiUrl: undefined, pubkeys: ["alice", "carol"] },
    { emoji: "👀", emojiUrl: undefined, pubkeys: ["bob"] },
  ]);
});

test("groupReactions keeps the first URL seen for an emoji", () => {
  const groups = groupReactions([
    { emoji: ":party:", pubkey: "alice" },
    { emoji: ":party:", emojiUrl: "https://x/party.png", pubkey: "bob" },
  ]);
  assert.equal(groups[0]?.emojiUrl, "https://x/party.png");
});

test("toggle adds a brand-new emoji group", () => {
  const groups = applyReactionToggle([], "🔥", "me", undefined);
  assert.deepEqual(groups, [
    { emoji: "🔥", emojiUrl: undefined, pubkeys: ["me"] },
  ]);
});

test("toggle joins an existing group", () => {
  const groups = applyReactionToggle(
    [{ emoji: "🔥", pubkeys: ["alice"] }],
    "🔥",
    "me",
  );
  assert.deepEqual(groups[0]?.pubkeys, ["alice", "me"]);
});

test("toggle removes my reaction and keeps others", () => {
  const groups = applyReactionToggle(
    [{ emoji: "🔥", pubkeys: ["alice", "me"] }],
    "🔥",
    "me",
  );
  assert.deepEqual(groups[0]?.pubkeys, ["alice"]);
});

test("toggle drops a group that empties", () => {
  const groups = applyReactionToggle(
    [
      { emoji: "🔥", pubkeys: ["me"] },
      { emoji: "👀", pubkeys: ["bob"] },
    ],
    "🔥",
    "me",
  );
  assert.deepEqual(
    groups.map((group) => group.emoji),
    ["👀"],
  );
});

const isAgent = (pubkey) => pubkey.startsWith("agent");

test("agent 👀 becomes a queued status, not a chip", () => {
  const { status, rest } = splitAgentStatus(
    [
      { emoji: "👀", pubkey: "agent-amp" },
      { emoji: "🔥", pubkey: "alice" },
    ],
    isAgent,
  );
  assert.deepEqual(status, { status: "queued", pubkeys: ["agent-amp"] });
  assert.deepEqual(
    rest.map((reaction) => reaction.emoji),
    ["🔥"],
  );
});

test("agent 💬 outranks 👀 and reports the working agents", () => {
  const { status } = splitAgentStatus(
    [
      { emoji: "👀", pubkey: "agent-amp" },
      { emoji: "💬", pubkey: "agent-codex" },
      { emoji: "💬", pubkey: "agent-codex" },
    ],
    isAgent,
  );
  assert.deepEqual(status, { status: "working", pubkeys: ["agent-codex"] });
});

test("human 👀/💬 stay ordinary chips", () => {
  const { status, rest } = splitAgentStatus(
    [
      { emoji: "👀", pubkey: "alice" },
      { emoji: "💬", pubkey: "bob" },
    ],
    isAgent,
  );
  assert.equal(status, null);
  assert.deepEqual(
    rest.map((reaction) => reaction.emoji),
    ["👀", "💬"],
  );
});

test("status emoji with a variation selector still matches", () => {
  const { status, rest } = splitAgentStatus(
    [{ emoji: "👀\uFE0F", pubkey: "agent-amp" }],
    isAgent,
  );
  assert.deepEqual(status, { status: "queued", pubkeys: ["agent-amp"] });
  assert.deepEqual(rest, []);
});

test("no reactions yields no status and no chips", () => {
  const { status, rest } = splitAgentStatus(undefined, isAgent);
  assert.equal(status, null);
  assert.deepEqual(rest, []);
});
