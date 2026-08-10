import assert from "node:assert/strict";
import { test } from "node:test";

import { buildNoteIndex, normalizeName, resolveWikilink } from "./noteIndex.ts";

const VAULT = "/vault";

test("normalizeName treats case, extension, dashes and underscores alike", () => {
  const canonical = normalizeName("Meeting Notes");
  for (const variant of [
    "meeting notes",
    "Meeting-Notes",
    "meeting_notes",
    "Meeting Notes.md",
    "MEETING-NOTES.MARKDOWN",
    "  meeting   notes  ",
  ]) {
    assert.equal(normalizeName(variant), canonical, variant);
  }
});

test("normalizeName keeps genuinely different names apart", () => {
  assert.notEqual(normalizeName("Meeting Notes"), normalizeName("Meetings"));
});

test("resolves a unique note by loose name", () => {
  const index = buildNoteIndex(VAULT, [`${VAULT}/Notes/Meeting Notes.md`]);
  for (const target of ["Meeting Notes", "meeting-notes", "MEETING_NOTES.md"]) {
    const resolved = resolveWikilink(target, `${VAULT}/other.md`, index);
    assert.deepEqual(
      resolved,
      { exists: true, path: `${VAULT}/Notes/Meeting Notes.md` },
      target,
    );
  }
});

test("prefers a note in the same folder as the linking file", () => {
  const index = buildNoteIndex(VAULT, [
    `${VAULT}/A/Shared.md`,
    `${VAULT}/B/Shared.md`,
  ]);
  assert.equal(
    resolveWikilink("Shared", `${VAULT}/B/other.md`, index).path,
    `${VAULT}/B/Shared.md`,
  );
  assert.equal(
    resolveWikilink("Shared", `${VAULT}/A/other.md`, index).path,
    `${VAULT}/A/Shared.md`,
  );
});

test("falls back to the shortest path when no sibling matches", () => {
  const index = buildNoteIndex(VAULT, [
    `${VAULT}/deep/nested/Shared.md`,
    `${VAULT}/Shared.md`,
  ]);
  assert.equal(
    resolveWikilink("Shared", `${VAULT}/elsewhere/other.md`, index).path,
    `${VAULT}/Shared.md`,
  );
});

test("ties break deterministically rather than by insertion order", () => {
  const paths = [`${VAULT}/b/Shared.md`, `${VAULT}/a/Shared.md`];
  const forward = buildNoteIndex(VAULT, paths);
  const reversed = buildNoteIndex(VAULT, [...paths].reverse());
  const from = `${VAULT}/z/other.md`;
  assert.equal(
    resolveWikilink("Shared", from, forward).path,
    resolveWikilink("Shared", from, reversed).path,
  );
});

test("a target containing a slash resolves as a vault-relative path", () => {
  const index = buildNoteIndex(VAULT, [
    `${VAULT}/Notes/Meeting Notes.md`,
    `${VAULT}/Archive/Meeting Notes.md`,
  ]);
  assert.equal(
    resolveWikilink("Archive/Meeting Notes", `${VAULT}/x.md`, index).path,
    `${VAULT}/Archive/Meeting Notes.md`,
  );
});

test("an unresolved target reports the path it would create", () => {
  const index = buildNoteIndex(VAULT, [`${VAULT}/Existing.md`]);
  const resolved = resolveWikilink("Brand New", `${VAULT}/x.md`, index);
  assert.deepEqual(resolved, { exists: false, path: `${VAULT}/Brand New.md` });

  const nested = resolveWikilink("Folder/Brand New", `${VAULT}/x.md`, index);
  assert.deepEqual(nested, {
    exists: false,
    path: `${VAULT}/Folder/Brand New.md`,
  });
});

test("returns null without an index or target", () => {
  const index = buildNoteIndex(VAULT, [`${VAULT}/A.md`]);
  assert.equal(resolveWikilink("A", `${VAULT}/x.md`, null), null);
  assert.equal(resolveWikilink("   ", `${VAULT}/x.md`, index), null);
});
