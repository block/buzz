import assert from "node:assert/strict";
import test from "node:test";

import { transcriptDiff } from "./transcriptDiff.ts";

test("identical decode is a no-op", () => {
  assert.deepEqual(transcriptDiff("hello world", "hello world"), {
    keep: 11,
    deleteLen: 0,
    insert: "",
  });
});

test("append-only decode inserts just the tail", () => {
  assert.deepEqual(transcriptDiff("hello", "hello world"), {
    keep: 5,
    deleteLen: 0,
    insert: " world",
  });
});

test("mid-phrase revision rewrites from the divergence point", () => {
  assert.deepEqual(transcriptDiff("space batoot", "space bar to"), {
    keep: 8,
    deleteLen: 4,
    insert: "r to",
  });
});

test("shrinking decode deletes the stale tail", () => {
  assert.deepEqual(transcriptDiff("hello worlds", "hello wor"), {
    keep: 9,
    deleteLen: 3,
    insert: "",
  });
});

test("first partial of a phrase inserts everything", () => {
  assert.deepEqual(transcriptDiff("", "hello"), {
    keep: 0,
    deleteLen: 0,
    insert: "hello",
  });
});

test("final commit appends only the trailing space", () => {
  assert.deepEqual(transcriptDiff("hello world", "hello world "), {
    keep: 11,
    deleteLen: 0,
    insert: " ",
  });
});
