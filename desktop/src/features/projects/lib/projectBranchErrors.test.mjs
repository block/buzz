import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isNoChannelBindingError,
  projectBranchErrorMessage,
} from "./projectBranchErrors.ts";

test("recognizes the relay's stable denial token", () => {
  // Body produced by the relay push policy (buzz-core
  // GIT_NO_CHANNEL_BINDING_BODY), as it arrives wrapped in git stderr.
  assert.ok(
    isNoChannelBindingError(
      "remote: no_channel_binding: repository has no channel binding\nerror: failed to push some refs",
    ),
  );
});

test("recognizes the legacy spaced phrase from older relays", () => {
  assert.ok(isNoChannelBindingError("push denied: no channel binding"));
});

test("does not match unrelated errors", () => {
  assert.ok(!isNoChannelBindingError("connection reset by peer"));
  assert.ok(!isNoChannelBindingError("no channel"));
});

test("maps binding denials to remediation copy", () => {
  const message = projectBranchErrorMessage(
    new Error("remote: no_channel_binding: repository has no channel binding"),
    "Failed to create branch.",
  );
  assert.ok(message.includes("buzz repos bind"));
});

test("passes through other errors and falls back for non-errors", () => {
  assert.equal(
    projectBranchErrorMessage(new Error("boom"), "fallback"),
    "boom",
  );
  assert.equal(projectBranchErrorMessage("boom", "fallback"), "fallback");
  assert.equal(projectBranchErrorMessage(null, "fallback"), "fallback");
});
