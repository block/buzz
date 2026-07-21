import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveSentMessageBody,
  shouldFetchSentMessage,
} from "./useSentMessageBody.ts";

const link = { channelId: "ch-1", messageId: "ev-1" };

test("shouldFetchSentMessage returns true when messageLink present and no inline content", () => {
  assert.equal(shouldFetchSentMessage(link, null), true);
});

test("shouldFetchSentMessage returns false when inline content is present", () => {
  assert.equal(shouldFetchSentMessage(link, "hello"), false);
});

test("shouldFetchSentMessage returns false when messageLink is null", () => {
  assert.equal(shouldFetchSentMessage(null, null), false);
});

test("resolveSentMessageBody returns inline content over fetched content", () => {
  assert.equal(
    resolveSentMessageBody("inline text", "fetched text"),
    "inline text",
  );
});

test("resolveSentMessageBody returns fetched content when no inline content", () => {
  assert.equal(resolveSentMessageBody(null, "fetched text"), "fetched text");
});

test("resolveSentMessageBody returns null when both inline and fetched are absent", () => {
  assert.equal(resolveSentMessageBody(null, undefined), null);
  assert.equal(resolveSentMessageBody(null, null), null);
});
