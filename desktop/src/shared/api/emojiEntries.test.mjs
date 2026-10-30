import assert from "node:assert/strict";
import test from "node:test";

import {
  EMOJI_ENTRY_RESOLVE_BATCH,
  EMOJI_ENTRY_RESOLVE_MAX_NAMES,
  fetchEmojiEntriesByName,
  parseEmojiEntryEvents,
} from "./emojiEntries.ts";

function entry({
  d,
  url = `https://x/${d}.png`,
  pubkey = "a",
  at = 1,
  id,
  tags,
}) {
  return {
    id: id ?? `${pubkey}-${d}-${at}`,
    pubkey,
    created_at: at,
    kind: 30180,
    content: d.replace(/[_-]/g, " "),
    tags: tags ?? [
      ["d", d],
      ["emoji", d, url],
    ],
    sig: "s",
  };
}

test("the newest event wins per shortcode and a second author is reported", () => {
  const warn = console.warn;
  console.warn = () => {};
  try {
    const { emoji, conflicts } = parseEmojiEntryEvents([
      entry({
        d: "party_parrot",
        url: "https://x/old.gif",
        at: 1,
        pubkey: "a",
      }),
      entry({
        d: "party_parrot",
        url: "https://x/new.gif",
        at: 5,
        pubkey: "b",
      }),
      entry({ d: "shipit", at: 2 }),
    ]);
    assert.deepEqual(emoji, [
      { shortcode: "party_parrot", url: "https://x/new.gif" },
      { shortcode: "shipit", url: "https://x/shipit.png" },
    ]);
    assert.deepEqual(conflicts, [
      { shortcode: "party_parrot", winnerPubkey: "b", otherPubkeys: ["a"] },
    ]);
  } finally {
    console.warn = warn;
  }
});

test("one author's replaced entry is not a conflict", () => {
  const { emoji, conflicts } = parseEmojiEntryEvents([
    entry({ d: "a1", url: "https://x/1.png", at: 1 }),
    entry({ d: "a1", url: "https://x/2.png", at: 2, id: "later" }),
  ]);
  assert.equal(emoji[0].url, "https://x/2.png");
  assert.deepEqual(conflicts, []);
});

test("events that break the contract are skipped", () => {
  const { emoji } = parseEmojiEntryEvents([
    { ...entry({ d: "ok" }), kind: 30030 },
    entry({ d: "bad", url: "javascript:alert(1)" }),
    entry({ d: "x", tags: [["d", "x"]] }),
    entry({
      d: "x",
      tags: [
        ["d", "x"],
        ["emoji", "y", "https://x/y.png"],
      ],
    }),
    entry({
      d: "x",
      tags: [
        ["d", "X"],
        ["emoji", "x", "https://x/y.png"],
      ],
    }),
    entry({ d: "fine" }),
  ]);
  assert.deepEqual(
    emoji.map((e) => e.shortcode),
    ["fine"],
  );
});

test("name resolve batches 50 names per #d filter and caps the total", async () => {
  const filters = [];
  const names = Array.from({ length: 130 }, (_, i) => `n${i}`);
  const result = await fetchEmojiEntriesByName(names, async (filter) => {
    filters.push(filter);
    return filter["#d"].map((d) => entry({ d }));
  });
  assert.equal(filters.length, 2);
  assert.deepEqual(
    filters.map((f) => f["#d"].length),
    [
      EMOJI_ENTRY_RESOLVE_BATCH,
      EMOJI_ENTRY_RESOLVE_MAX_NAMES - EMOJI_ENTRY_RESOLVE_BATCH,
    ],
  );
  assert.ok(filters.every((f) => f.kinds.length === 1 && f.kinds[0] === 30180));
  assert.equal(result.emoji.length, EMOJI_ENTRY_RESOLVE_MAX_NAMES);
});

test("a failed name query rejects", async () => {
  await assert.rejects(
    fetchEmojiEntriesByName(["a"], async () => {
      throw new Error("down");
    }),
    /down/,
  );
});
