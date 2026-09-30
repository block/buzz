import assert from "node:assert/strict";
import test from "node:test";

import { parseVideoReviewTimecode } from "./videoReviewTimecode.ts";

test("parseVideoReviewTimecode extracts supported leading timecodes", () => {
  assert.deepEqual(parseVideoReviewTimecode("[00:10.7] Tighten **this** cut"), {
    seconds: 10.7,
    text: "Tighten **this** cut",
    timecode: "00:10.7",
  });
  assert.deepEqual(parseVideoReviewTimecode("[1:02:03] Long-form note"), {
    seconds: 3723,
    text: "Long-form note",
    timecode: "1:02:03",
  });
});

test("parseVideoReviewTimecode ignores ordinary bracketed markdown", () => {
  assert.equal(parseVideoReviewTimecode("[docs](https://example.com)"), null);
  assert.equal(parseVideoReviewTimecode("Comment at [00:10]"), null);
});
