import assert from "node:assert/strict";
import { test } from "node:test";

import { loadPdfPreview, resetPdfPreviewCache } from "./pdfDocument.ts";

// No Tauri bridge exists under node, so every preview load rejects at the
// native byte fetch — enough to exercise the cache's sharing, failure
// eviction, and community-reset behavior on the production code path.
const HREF = `https://relay.example/media/${"a".repeat(64)}.pdf`;

test("loadPdfPreview: concurrent cards share one in-flight render", async () => {
  resetPdfPreviewCache();
  const first = loadPdfPreview(HREF, 384);
  const second = loadPdfPreview(HREF, 384);
  assert.equal(first, second);
  await assert.rejects(first);
});

test("loadPdfPreview: a failed render is evicted so the next mount retries", async () => {
  resetPdfPreviewCache();
  const failed = loadPdfPreview(HREF, 384);
  await assert.rejects(failed);
  const retry = loadPdfPreview(HREF, 384);
  assert.notEqual(retry, failed);
  await assert.rejects(retry);
});

test("resetPdfPreviewCache: community switch drops cached previews", async () => {
  resetPdfPreviewCache();
  const before = loadPdfPreview(HREF, 384);
  resetPdfPreviewCache();
  const after = loadPdfPreview(HREF, 384);
  assert.notEqual(after, before);
  await Promise.allSettled([before, after]);
});
