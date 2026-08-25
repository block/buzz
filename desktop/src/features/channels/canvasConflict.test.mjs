import assert from "node:assert/strict";
import test from "node:test";

import {
  CANVAS_EXPECTED_REVISION_NONE,
  isCanvasConflictError,
} from "./canvasConflict.ts";

// The two frozen conflict strings are both conflicts from the user's
// perspective: the head moved, or the revision the client expected no longer
// exists. The desktop `set_canvas` command produces these client-side. The
// helper must recognize each whether it arrives as an Error or a raw string
// (the Tauri IPC layer hands back either), and must not misfire on unrelated
// errors.

test("head-moved reject is a conflict as Error and as raw string", () => {
  const message = "conflict: canvas changed since it was loaded";
  assert.equal(isCanvasConflictError(new Error(message)), true);
  assert.equal(isCanvasConflictError(message), true);
});

test("revision-does-not-exist reject is a conflict as Error and as raw string", () => {
  const message = "conflict: canvas revision does not exist";
  assert.equal(isCanvasConflictError(new Error(message)), true);
  assert.equal(isCanvasConflictError(message), true);
});

test("conflict marker embedded in a longer wrapped message still matches", () => {
  const wrapped = new Error(
    "submit failed: conflict: canvas revision does not exist (relay)",
  );
  assert.equal(isCanvasConflictError(wrapped), true);
});

test("unrelated errors are not conflicts", () => {
  assert.equal(isCanvasConflictError(new Error("relay unreachable")), false);
  assert.equal(isCanvasConflictError("some other failure"), false);
  assert.equal(isCanvasConflictError(null), false);
  assert.equal(isCanvasConflictError(undefined), false);
  assert.equal(
    isCanvasConflictError({
      message: "conflict: canvas changed since it was loaded",
    }),
    false,
  );
});

test("the create-race sentinel is the literal contract value", () => {
  assert.equal(CANVAS_EXPECTED_REVISION_NONE, "none");
});
