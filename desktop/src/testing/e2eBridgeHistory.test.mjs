import assert from "node:assert/strict";
import test from "node:test";
import { selectMockHistory } from "./e2eBridgeHistory.ts";

const event = (id, created_at, overrides = {}) => ({
  id,
  created_at,
  kind: 9,
  pubkey: "author",
  content: id,
  sig: "",
  tags: [
    ["h", "channel"],
    ["e", "root"],
  ],
  ...overrides,
});
const rows = [event("a", 10), event("b", 20), event("c", 20), event("d", 30)];
const ids = (filters, events = rows) =>
  selectMockHistory(new Map([["channel", events]]), filters).map((e) => e.id);

test("replay includes admission-gap events with inclusive time bounds and relay ordering", () => {
  assert.deepEqual(
    ids([{ "#h": ["channel"], since: 10, until: 20, limit: 2 }]),
    ["b", "c"],
  );
  assert.deepEqual(ids([{ since: 20, limit: 2 }]), ["b", "d"]);
  assert.deepEqual(ids([{ until: 0 }]), []);
});
test("live-only requests do not replay and old synthetic live events stay outside since", () => {
  assert.deepEqual(ids([{ limit: 0 }]), []);
  assert.deepEqual(ids([{ since: 31 }]), []);
});
test("filters retain author, kind, id and tag constraints", () => {
  for (const filter of [
    { authors: ["other"] },
    { kinds: [7] },
    { ids: ["absent"] },
    { "#h": ["other"] },
    { "#e": ["other"] },
  ]) {
    assert.deepEqual(ids([filter]), []);
  }
  assert.deepEqual(
    ids([{ authors: ["author"], kinds: [9], ids: ["b"], "#e": ["root"] }]),
    ["b"],
  );
});
test("multiple filters apply independent limits and deduplicate their union", () => {
  assert.deepEqual(
    ids([
      { until: 20, limit: 1 },
      { since: 20, limit: 2 },
    ]),
    ["b", "d"],
  );
  assert.deepEqual(ids([{ limit: 10 }], [...rows, rows[0]]), [
    "a",
    "b",
    "c",
    "d",
  ]);
});

test("channel ownership scopes untagged auxiliary events without rewriting wire tags", () => {
  const reaction = event("reaction", 20, { kind: 7, tags: [["e", "root"]] });
  const foreign = event("foreign", 20, { tags: [["e", "root"]] });
  const conflicting = event("conflicting", 20, {
    kind: 7,
    tags: [
      ["e", "root"],
      ["h", "other"],
    ],
  });
  const channels = new Map([
    ["channel", [reaction, conflicting]],
    ["other", [foreign]],
  ]);
  assert.deepEqual(
    selectMockHistory(channels, [{ "#h": ["channel"], "#e": ["root"] }]),
    [reaction],
  );
  assert.deepEqual(reaction.tags, [["e", "root"]]);
});
