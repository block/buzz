import assert from "node:assert/strict";
import test from "node:test";

import {
  buildSentFromThreadTag,
  getSentFromThreadReference,
  getSentFromThreadRootId,
  SENT_FROM_THREAD_TAG,
  summarizeThreadRoot,
} from "./sentFromThread.ts";

test("buildSentFromThreadTag records the normalized root event ID", () => {
  assert.deepEqual(buildSentFromThreadTag("  root-event  "), [
    SENT_FROM_THREAD_TAG,
    "root-event",
  ]);
});

test("buildSentFromThreadTag includes a normalized human excerpt", () => {
  assert.deepEqual(buildSentFromThreadTag("root-event", "  Launch plan  "), [
    SENT_FROM_THREAD_TAG,
    "root-event",
    "Launch plan",
  ]);
});

test("buildSentFromThreadTag rejects an empty root event ID", () => {
  assert.throws(
    () => buildSentFromThreadTag("  "),
    /thread root event ID is required/,
  );
});

test("sent-from-thread references accept an optional excerpt", () => {
  assert.equal(
    getSentFromThreadRootId([
      ["h", "channel-id"],
      [SENT_FROM_THREAD_TAG, "root-event"],
    ]),
    "root-event",
  );
  assert.deepEqual(
    getSentFromThreadReference([
      [SENT_FROM_THREAD_TAG, "root-event", "Root summary"],
    ]),
    { rootEventId: "root-event", rootExcerpt: "Root summary" },
  );
  assert.equal(
    getSentFromThreadRootId([
      [SENT_FROM_THREAD_TAG, "root-event", "summary", "extra"],
    ]),
    null,
  );
  assert.equal(getSentFromThreadRootId([[SENT_FROM_THREAD_TAG, "  "]]), null);
  assert.equal(getSentFromThreadRootId(undefined), null);
});

test("summarizeThreadRoot keeps concise text and ignores media-only roots", () => {
  assert.equal(summarizeThreadRoot("  **Launch**   plan  "), "Launch plan");
  assert.equal(
    summarizeThreadRoot("![diagram](https://example.com/diagram.png)"),
    null,
  );
  assert.equal(
    summarizeThreadRoot("See [the plan](https://example.com/plan) for details"),
    "See the plan for details",
  );
  assert.match(summarizeThreadRoot("word ".repeat(30)) ?? "", /…$/);
});

test("summarizeThreadRoot preserves Unicode boundaries and redacts spoilers", () => {
  const summary = summarizeThreadRoot(`${"a".repeat(62)}😀 more`);
  assert.equal(summary, `${"a".repeat(62)}😀…`);
  assert.equal(
    summarizeThreadRoot("Public ||confidential details|| update"),
    "Public update",
  );
});
