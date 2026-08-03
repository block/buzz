import assert from "node:assert/strict";
import test from "node:test";

import {
  appendTranscribedText,
  getDictationSendDecision,
  shouldAutoSubmitDictation,
} from "./voiceInput.ts";

test("dictation send stops capture before submitting", () => {
  assert.equal(
    getDictationSendDecision({
      isRecording: true,
      isStarting: false,
      isTranscribing: true,
    }),
    "stop-recording",
  );
  assert.equal(
    getDictationSendDecision({
      isRecording: false,
      isStarting: true,
      isTranscribing: false,
    }),
    "stop-recording",
  );
});

test("dictation send waits for the final transcript", () => {
  assert.equal(
    getDictationSendDecision({
      isRecording: false,
      isStarting: false,
      isTranscribing: true,
    }),
    "wait",
  );
  assert.equal(
    getDictationSendDecision({
      isRecording: false,
      isStarting: false,
      isTranscribing: false,
    }),
    "send",
  );
});

test("dictation auto-submit waits until capture and transcription settle", () => {
  const state = {
    requested: true,
    isRecording: false,
    isStarting: false,
    isTranscribing: false,
  };
  assert.equal(shouldAutoSubmitDictation(state), true);
  assert.equal(
    shouldAutoSubmitDictation({ ...state, requested: false }),
    false,
  );
  assert.equal(
    shouldAutoSubmitDictation({ ...state, isRecording: true }),
    false,
  );
  assert.equal(
    shouldAutoSubmitDictation({ ...state, isStarting: true }),
    false,
  );
  assert.equal(
    shouldAutoSubmitDictation({ ...state, isTranscribing: true }),
    false,
  );
});

// ── appendTranscribedText ───────────────────────────────────────────────────

test("appendTranscribedText_appendsToExistingText", () => {
  assert.equal(appendTranscribedText("Hello", "world"), "Hello world");
});

test("appendTranscribedText_appendsToEmptyBase", () => {
  assert.equal(appendTranscribedText("", "hello"), "hello");
});

test("appendTranscribedText_avoidsSpaceBeforePunctuation", () => {
  assert.equal(appendTranscribedText("Hello", ", world"), "Hello, world");
});
