import assert from "node:assert/strict";
import test from "node:test";

import {
  backgroundMediaUploadPhaseLabel,
  isNativeMediaUploadPhase,
  resolveBackgroundMediaUploadPhase,
} from "./backgroundMediaUploadPhase.ts";

test("upload phase labels describe the real work in plain language", () => {
  assert.equal(backgroundMediaUploadPhaseLabel("preparing"), "Preparing");
  assert.equal(
    backgroundMediaUploadPhaseLabel("processing-video"),
    "Processing",
  );
  assert.equal(
    backgroundMediaUploadPhaseLabel("converting-image"),
    "Converting",
  );
  assert.equal(
    backgroundMediaUploadPhaseLabel("processing-files"),
    "Processing",
  );
  assert.equal(backgroundMediaUploadPhaseLabel("uploading"), "Uploading");
  assert.equal(backgroundMediaUploadPhaseLabel("finishing"), "Finishing");
});

test("upload phase validation accepts only phases emitted by native code", () => {
  assert.equal(isNativeMediaUploadPhase("processing-video"), true);
  assert.equal(isNativeMediaUploadPhase("processing-files"), false);
  assert.equal(isNativeMediaUploadPhase("transcribing"), false);
  assert.equal(isNativeMediaUploadPhase(null), false);
});

test("upload phase aggregation favors active transfer and combines mixed work", () => {
  assert.equal(resolveBackgroundMediaUploadPhase([]), "preparing");
  assert.equal(
    resolveBackgroundMediaUploadPhase(["processing-video", "uploading"]),
    "uploading",
  );
  assert.equal(
    resolveBackgroundMediaUploadPhase(["processing-video", "preparing"]),
    "processing-files",
  );
  assert.equal(
    resolveBackgroundMediaUploadPhase(["finishing", "finishing"]),
    "finishing",
  );
});
