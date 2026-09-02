import assert from "node:assert/strict";
import test from "node:test";

import {
  formatMessageSendError,
  getErrorMessage,
  MENTION_ADMISSION_MAX_AGE_MS,
  mergeMentionRecipients,
  shouldRevalidateMentionsAtPublish,
} from "./useMentionSendFlow.helpers.ts";

const fastPathSend = {
  hasDeferredUpload: false,
  hasDeferredLinkPreviews: false,
  relaySideEffectsRan: false,
  msSinceAdmission: 3,
};

test("formatMessageSendError preserves the publication failure", () => {
  assert.equal(
    formatMessageSendError(new Error("relay rejected voice note")),
    "Message failed to send: relay rejected voice note",
  );
});

test("getErrorMessage preserves Tauri string errors", () => {
  assert.equal(
    getErrorMessage(
      "relay returned 415 Unsupported Media Type",
      "Unknown error",
    ),
    "relay returned 415 Unsupported Media Type",
  );
  assert.equal(
    getErrorMessage({ message: "upload rejected" }, "Unknown error"),
    "upload rejected",
  );
  assert.equal(getErrorMessage({}, "Unknown error"), "Unknown error");
});

test("the immediate fast path reuses the admitted mention set", () => {
  assert.equal(shouldRevalidateMentionsAtPublish(fastPathSend), false);
});

test("a long gap between admission and publish forces a fresh pass", () => {
  // The staleness bound, not any named trigger: none of the enumerated
  // relay-touching steps ran, yet the two passes are far enough apart that
  // the earlier admission can no longer be assumed current.
  assert.equal(
    shouldRevalidateMentionsAtPublish({
      ...fastPathSend,
      msSinceAdmission: MENTION_ADMISSION_MAX_AGE_MS,
    }),
    true,
  );
  assert.equal(
    shouldRevalidateMentionsAtPublish({
      ...fastPathSend,
      msSinceAdmission: MENTION_ADMISSION_MAX_AGE_MS - 1,
    }),
    false,
  );
});

test("each relay-touching trigger forces a fresh pass on its own", () => {
  for (const trigger of [
    "hasDeferredUpload",
    "hasDeferredLinkPreviews",
    "relaySideEffectsRan",
  ]) {
    assert.equal(
      shouldRevalidateMentionsAtPublish({ ...fastPathSend, [trigger]: true }),
      true,
      `${trigger} must revalidate even inside the staleness bound`,
    );
  }
});

test("address-locked agents join explicit mentions without duplicating recipients", () => {
  const explicit = ["A".repeat(64), "b".repeat(64)];
  const locked = ["a".repeat(64), "C".repeat(64)];

  assert.deepEqual(mergeMentionRecipients(explicit, locked), [
    "a".repeat(64),
    "b".repeat(64),
    "c".repeat(64),
  ]);
});
