import assert from "node:assert/strict";
import test from "node:test";

import { buildProductFeedbackEvent } from "./useSendFeedback.ts";

test("buildProductFeedbackEvent uses body and category tag", () => {
  assert.deepEqual(
    buildProductFeedbackEvent({ category: "bug", message: "  It broke  " }, []),
    { content: "It broke", tags: [["category", "bug"]] },
  );
});

test("buildProductFeedbackEvent omits absent category and retains imeta", () => {
  const attachment = {
    url: "https://example.test/screenshot.png",
    sha256: "ab".repeat(32),
    size: 42,
    type: "image/png",
    uploaded: 42,
  };
  const result = buildProductFeedbackEvent(
    { category: null, message: "Useful feedback" },
    [attachment],
  );
  assert.match(result.content, /Useful feedback/);
  assert.equal(
    result.tags.some((tag) => tag[0] === "category"),
    false,
  );
  assert.equal(
    result.tags.some((tag) => tag[0] === "imeta"),
    true,
  );
});
