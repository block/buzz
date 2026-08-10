import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  cacheDocument,
  clearDocumentCache,
  forgetCachedDocument,
  getCachedDocument,
} from "./documentJsonCache.ts";

const doc = (text) => ({
  content: [{ content: [{ text, type: "text" }], type: "paragraph" }],
  type: "doc",
});

beforeEach(() => {
  clearDocumentCache();
});

test("a document is returned only for the markdown it was parsed from", () => {
  cacheDocument("/vault/a.md", "# One", doc("One"));

  assert.deepEqual(getCachedDocument("/vault/a.md", "# One"), doc("One"));
  // This is the whole safety argument: any difference in the source text is a
  // miss, so a stale entry cannot be installed into the editor.
  assert.equal(getCachedDocument("/vault/a.md", "# One edited"), null);
  assert.equal(getCachedDocument("/vault/b.md", "# One"), null);
});

test("re-caching a path replaces its entry rather than accumulating", () => {
  cacheDocument("/vault/a.md", "# One", doc("One"));
  cacheDocument("/vault/a.md", "# Two", doc("Two"));

  assert.equal(getCachedDocument("/vault/a.md", "# One"), null);
  assert.deepEqual(getCachedDocument("/vault/a.md", "# Two"), doc("Two"));
});

test("forgetting a path drops just that entry", () => {
  cacheDocument("/vault/a.md", "a", doc("a"));
  cacheDocument("/vault/b.md", "b", doc("b"));

  forgetCachedDocument("/vault/a.md");

  assert.equal(getCachedDocument("/vault/a.md", "a"), null);
  assert.deepEqual(getCachedDocument("/vault/b.md", "b"), doc("b"));
});

test("clearing drops everything, as a vault switch requires", () => {
  cacheDocument("/vault/a.md", "a", doc("a"));
  cacheDocument("/vault/b.md", "b", doc("b"));

  clearDocumentCache();

  assert.equal(getCachedDocument("/vault/a.md", "a"), null);
  assert.equal(getCachedDocument("/vault/b.md", "b"), null);
});

test("the cache is bounded, evicting least-recently-used entries", () => {
  for (let i = 0; i < 20; i += 1) {
    cacheDocument(`/vault/${i}.md`, `note ${i}`, doc(`note ${i}`));
  }

  // The oldest are gone; the most recent survive.
  assert.equal(getCachedDocument("/vault/0.md", "note 0"), null);
  assert.deepEqual(
    getCachedDocument("/vault/19.md", "note 19"),
    doc("note 19"),
  );
});

test("reading an entry keeps it in the working set", () => {
  cacheDocument("/vault/keep.md", "keep", doc("keep"));

  // Fill past the cap, touching `keep.md` along the way so it stays hot.
  for (let i = 0; i < 20; i += 1) {
    assert.deepEqual(getCachedDocument("/vault/keep.md", "keep"), doc("keep"));
    cacheDocument(`/vault/${i}.md`, `note ${i}`, doc(`note ${i}`));
  }

  assert.deepEqual(getCachedDocument("/vault/keep.md", "keep"), doc("keep"));
});
