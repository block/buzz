import assert from "node:assert/strict";
import test from "node:test";

import {
  parseCaptionLanguageBanner,
  shouldSpeakCaption,
} from "./captionLanguage.ts";

test("parses a recognized [XX] banner as lowercase", () => {
  assert.equal(parseCaptionLanguageBanner("[ES] Hola"), "es");
  assert.equal(parseCaptionLanguageBanner("[zh] 你好"), "zh");
});

test("returns null for content with no banner", () => {
  assert.equal(parseCaptionLanguageBanner("Hello there"), null);
});

test("speak-aloud toggle off silences every caption, banner or not", () => {
  assert.equal(shouldSpeakCaption("[ES] Hola", false, "es"), false);
  assert.equal(shouldSpeakCaption("Hello there", false, "en"), false);
});

test("untagged messages are always eligible to speak", () => {
  assert.equal(shouldSpeakCaption("Hello there", true, "es"), true);
});

test("banner-tagged captions only speak when they match the listener's language", () => {
  assert.equal(shouldSpeakCaption("[ES] Hola", true, "es"), true);
  assert.equal(shouldSpeakCaption("[ES] Hola", true, "en"), false);
});
