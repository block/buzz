import assert from "node:assert/strict";
import test from "node:test";

import { initializeI18n } from "@/i18n/index.ts";
import { formatHuddleActionError } from "./huddleError.ts";

// `formatHuddleActionError` resolves its copy through `i18n.t`, which returns
// `undefined` until the singleton boots (`main.tsx` does that in the app).
// English is pinned explicitly before init: node's own `navigator.languages`
// reports the host system locale — which may be zh-CN — and every assertion
// below is the English contract.
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();

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
