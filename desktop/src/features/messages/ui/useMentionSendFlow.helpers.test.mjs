import assert from "node:assert/strict";
import test from "node:test";

import {
  formatMessageSendError,
  getErrorMessage,
  mergeMentionRecipients,
  mentionRevalidationOptions,
} from "./useMentionSendFlow.helpers.ts";

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

test("address-locked agents join explicit mentions without duplicating recipients", () => {
  const explicit = ["A".repeat(64), "b".repeat(64)];
  const locked = ["a".repeat(64), "C".repeat(64)];

  assert.deepEqual(mergeMentionRecipients(explicit, locked), [
    "a".repeat(64),
    "b".repeat(64),
    "c".repeat(64),
  ]);
});

test("revalidation carries captured and prepared agent keys independently of the cleared composer", () => {
  const draft = {
    inlineAgentMentionPubkeys: ["A".repeat(64)],
    addressedAgentPubkeys: ["b".repeat(64)],
  };
  assert.deepEqual(mentionRevalidationOptions(draft, "prepare"), {
    phase: "prepare",
    intendedAgentPubkeys: ["a".repeat(64), "b".repeat(64)],
  });
  assert.deepEqual(
    mentionRevalidationOptions(draft, "publish", [
      "a".repeat(64),
      "c".repeat(64),
    ]),
    {
      phase: "publish",
      intendedAgentPubkeys: ["a".repeat(64), "b".repeat(64), "c".repeat(64)],
    },
  );
});
