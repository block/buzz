import assert from "node:assert/strict";
import test from "node:test";

import { getErrorMessage } from "./useMentionSendFlow.helpers.ts";

const fallback = "Unknown error";

test("preserves an actionable raw ffmpeg error string", () => {
  const error =
    "ffmpeg is required for video uploads but was not found.\n\nInstall it:\n  macOS: brew install ffmpeg";

  assert.equal(getErrorMessage(error, fallback), error);
});

test("preserves an ordinary Error message", () => {
  assert.equal(
    getErrorMessage(new Error("Video conversion failed"), fallback),
    "Video conversion failed",
  );
});

test("uses the fallback for empty and whitespace-only text", () => {
  for (const error of ["", " \n\t ", new Error(""), new Error(" \n\t ")]) {
    assert.equal(getErrorMessage(error, fallback), fallback);
  }
});

test("uses the fallback for non-text rejection values", () => {
  for (const error of [
    null,
    undefined,
    0,
    42,
    false,
    true,
    [],
    ["Video conversion failed"],
    {},
    { message: "Video conversion failed" },
  ]) {
    assert.equal(getErrorMessage(error, fallback), fallback);
  }
});

test("preserves original multiline and surrounding whitespace after validation", () => {
  const rawString = " \nVideo conversion failed\nTry another file.\n ";
  const errorMessage = "\tVideo conversion failed\nDecoder unavailable\t";

  assert.equal(getErrorMessage(rawString, fallback), rawString);
  assert.equal(
    getErrorMessage(new Error(errorMessage), fallback),
    errorMessage,
  );
});
