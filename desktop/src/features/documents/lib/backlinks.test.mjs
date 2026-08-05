import assert from "node:assert/strict";
import { test } from "node:test";

import { getBacklinks, groupMentionsBySource } from "./backlinks.ts";
import { buildNoteIndex } from "./noteIndex.ts";

const VAULT = "/vault";
const TARGET = `${VAULT}/Meeting Notes.md`;

function fixture(entries) {
  const contents = new Map(entries);
  const index = buildNoteIndex(VAULT, [TARGET, ...contents.keys()]);
  return { contents, index };
}

test("finds a linked mention and reports its line", () => {
  const { contents, index } = fixture([
    [`${VAULT}/Daily.md`, "Morning.\n\nSee [[Meeting Notes]] for details.\n"],
  ]);
  const { linked, unlinked } = getBacklinks({
    contents,
    index,
    targetPath: TARGET,
  });

  assert.equal(linked.length, 1);
  assert.equal(linked[0].sourcePath, `${VAULT}/Daily.md`);
  assert.equal(linked[0].sourceName, "Daily");
  assert.equal(linked[0].lineNumber, 3);
  assert.match(linked[0].line, /See \[\[Meeting Notes\]\]/);
  assert.equal(unlinked.length, 0);
});

test("matches loose name variants through the index", () => {
  const { contents, index } = fixture([
    [`${VAULT}/A.md`, "[[meeting-notes]]"],
    [`${VAULT}/B.md`, "[[MEETING_NOTES]]"],
  ]);
  const { linked } = getBacklinks({ contents, index, targetPath: TARGET });
  assert.deepEqual(linked.map((m) => m.sourceName).sort(), ["A", "B"]);
});

test("finds unlinked mentions and keeps them separate", () => {
  const { contents, index } = fixture([
    [`${VAULT}/Plain.md`, "I discussed Meeting Notes with the team.\n"],
  ]);
  const { linked, unlinked } = getBacklinks({
    contents,
    index,
    targetPath: TARGET,
  });

  assert.equal(linked.length, 0);
  assert.equal(unlinked.length, 1);
  assert.equal(unlinked[0].kind, "unlinked");
  assert.equal(unlinked[0].sourceName, "Plain");
});

test("a linked line is not also reported as unlinked", () => {
  // The name appears inside the wikilink; counting it twice would double every
  // backlink in the panel.
  const { contents, index } = fixture([
    [`${VAULT}/A.md`, "See [[Meeting Notes]] today.\n"],
  ]);
  const { linked, unlinked } = getBacklinks({
    contents,
    index,
    targetPath: TARGET,
  });
  assert.equal(linked.length, 1);
  assert.equal(unlinked.length, 0);
});

test("unlinked matching respects word boundaries", () => {
  const { contents, index } = fixture([
    [`${VAULT}/A.md`, "Meeting Notesworthy things happened.\n"],
  ]);
  const { unlinked } = getBacklinks({ contents, index, targetPath: TARGET });
  assert.equal(
    unlinked.length,
    0,
    "a substring inside a longer word is not a mention",
  );
});

test("unlinked matching is case-insensitive", () => {
  const { contents, index } = fixture([
    [`${VAULT}/A.md`, "we reviewed meeting notes yesterday\n"],
  ]);
  const { unlinked } = getBacklinks({ contents, index, targetPath: TARGET });
  assert.equal(unlinked.length, 1);
});

test("a note is never its own backlink", () => {
  const contents = new Map([
    [TARGET, "# Meeting Notes\n\nMeeting Notes again.\n"],
  ]);
  const index = buildNoteIndex(VAULT, [TARGET]);
  const { linked, unlinked } = getBacklinks({
    contents,
    index,
    targetPath: TARGET,
  });
  assert.equal(linked.length, 0);
  assert.equal(unlinked.length, 0);
});

test("a link to a different note is not a backlink here", () => {
  const { contents, index } = fixture([
    [`${VAULT}/A.md`, "See [[Something Else]].\n"],
  ]);
  const { linked } = getBacklinks({ contents, index, targetPath: TARGET });
  assert.equal(linked.length, 0);
});

test("works without an index by comparing names", () => {
  // Backlinks stay useful while the corpus is still loading.
  const contents = new Map([[`${VAULT}/A.md`, "See [[Meeting Notes]].\n"]]);
  const { linked } = getBacklinks({
    contents,
    index: null,
    targetPath: TARGET,
  });
  assert.equal(linked.length, 1);
});

test("groups multiple mentions from one note together", () => {
  const { contents, index } = fixture([
    [`${VAULT}/A.md`, "[[Meeting Notes]]\n\nand [[Meeting Notes]] again\n"],
    [`${VAULT}/B.md`, "[[Meeting Notes]]\n"],
  ]);
  const { linked } = getBacklinks({ contents, index, targetPath: TARGET });
  const groups = groupMentionsBySource(linked);

  assert.equal(groups.length, 2);
  const a = groups.find((g) => g.sourceName === "A");
  assert.equal(a.mentions.length, 2);
});
