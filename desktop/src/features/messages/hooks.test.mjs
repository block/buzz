import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { shouldUseNativeAllMentionTransport } from "./hooks.ts";

const detectionCases = JSON.parse(
  await readFile(
    new URL(
      "../../../../crates/buzz-sdk/tests/fixtures/at_all_detection.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

test("the transport gate has no false negatives against SDK-positive cases", () => {
  for (const detectionCase of detectionCases.filter(({ active }) => active)) {
    assert.equal(
      shouldUseNativeAllMentionTransport(detectionCase.content),
      true,
      detectionCase.name,
    );
  }
});

test("ordinary plain text retains the WebSocket-eligible path", () => {
  assert.equal(shouldUseNativeAllMentionTransport("plain text"), false);
});
