import assert from "node:assert/strict";
import test from "node:test";

import { formatHuddleActionError } from "./huddleError.ts";

const AUDIO_UNAVAILABLE_MESSAGE =
  "Huddle audio isn’t available on this server. Ask an administrator to turn it on.";

test("maps the relay deployment rejection to actionable copy", () => {
  assert.equal(
    formatHuddleActionError(
      "audio relay auth error: huddle audio unavailable in this deployment",
      "join",
    ),
    AUDIO_UNAVAILABLE_MESSAGE,
  );
});

test("recognizes the relay error code when present", () => {
  assert.equal(
    formatHuddleActionError("huddle_audio_unavailable", "start"),
    AUDIO_UNAVAILABLE_MESSAGE,
  );
});

test("preserves other string and Error messages", () => {
  assert.equal(
    formatHuddleActionError("Microphone unavailable", "join"),
    "Microphone unavailable",
  );
  assert.equal(
    formatHuddleActionError(new Error("Connection timed out"), "start"),
    "Connection timed out",
  );
});

test("uses action-specific fallback copy for unknown errors", () => {
  assert.equal(
    formatHuddleActionError({ reason: "unknown" }, "join"),
    "Couldn’t join the huddle.",
  );
  assert.equal(
    formatHuddleActionError(null, "start"),
    "Couldn’t start the huddle.",
  );
});
