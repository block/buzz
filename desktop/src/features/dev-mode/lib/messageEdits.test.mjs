import assert from "node:assert/strict";
import test from "node:test";

import { applyMessageEdits, collectMessageEdits } from "./messageEdits.ts";

const KIND_MESSAGE = 9;
const KIND_EDIT = 40003;

let counter = 0;
function message({ id, content, tags = [], createdAt }) {
  counter += 1;
  return {
    id,
    pubkey: "author",
    created_at: createdAt ?? counter,
    kind: KIND_MESSAGE,
    tags,
    content,
    sig: "",
  };
}

function edit({ targetId, content, tags = [], createdAt }) {
  counter += 1;
  return {
    id: `edit-${counter}`,
    pubkey: "author",
    created_at: createdAt ?? counter,
    kind: KIND_EDIT,
    tags: [["e", targetId], ...tags],
    content,
    sig: "",
  };
}

test("applyMessageEdits_replacesContentWithTheLatestEdit", () => {
  const events = [
    message({ id: "m1", content: "orig", createdAt: 1 }),
    edit({ targetId: "m1", content: "first edit", createdAt: 2 }),
    edit({ targetId: "m1", content: "second edit", createdAt: 3 }),
  ];
  const result = applyMessageEdits(events);
  assert.equal(result[0].content, "second edit");
});

test("applyMessageEdits_latestWinsRegardlessOfArrayOrder", () => {
  const events = [
    edit({ targetId: "m1", content: "newer", createdAt: 5 }),
    edit({ targetId: "m1", content: "older", createdAt: 2 }),
    message({ id: "m1", content: "orig", createdAt: 1 }),
  ];
  const result = applyMessageEdits(events);
  assert.equal(result[2].content, "newer");
});

test("applyMessageEdits_leavesUneditedEventsUntouched", () => {
  const original = message({ id: "m1", content: "orig" });
  const other = message({ id: "m2", content: "other" });
  const result = applyMessageEdits([
    original,
    other,
    edit({ targetId: "m2", content: "edited" }),
  ]);
  assert.equal(result[0], original);
  assert.equal(result[1].content, "edited");
});

test("applyMessageEdits_overlaysImetaTagsFromTheEdit", () => {
  const events = [
    message({
      id: "m1",
      content: "orig",
      tags: [
        ["h", "channel"],
        ["imeta", "url https://old.example/a.png", "m image/png"],
      ],
    }),
    edit({
      targetId: "m1",
      content: "edited",
      tags: [["imeta", "url https://new.example/b.png", "m image/png"]],
    }),
  ];
  const [overlaid] = applyMessageEdits(events);
  const imeta = overlaid.tags.filter((tag) => tag[0] === "imeta");
  assert.equal(imeta.length, 1);
  assert.equal(imeta[0][1], "url https://new.example/b.png");
  // Non-imeta tags stay from the original.
  assert.deepEqual(
    overlaid.tags.find((tag) => tag[0] === "h"),
    ["h", "channel"],
  );
});

test("applyMessageEdits_noEditsReturnsEventsUnchanged", () => {
  const events = [message({ id: "m1", content: "orig" })];
  assert.deepEqual(applyMessageEdits(events), events);
  assert.deepEqual(applyMessageEdits(undefined), []);
});

test("collectMessageEdits_mapsTargetToLatestEdit", () => {
  const events = [
    message({ id: "m1", content: "orig" }),
    edit({ targetId: "m1", content: "one", createdAt: 10 }),
    edit({ targetId: "m1", content: "two", createdAt: 20 }),
    edit({ targetId: "m9", content: "other", createdAt: 5 }),
  ];
  const edits = collectMessageEdits(events);
  assert.equal(edits.size, 2);
  assert.equal(edits.get("m1")?.content, "two");
  assert.equal(edits.get("m9")?.content, "other");
});
