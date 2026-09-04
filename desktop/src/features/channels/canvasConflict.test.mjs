import assert from "node:assert/strict";
import test from "node:test";

import {
  CANVAS_EXPECTED_REVISION_NONE,
  CANVAS_CONFLICT_MESSAGE,
  CANVAS_SUPERSEDED_MESSAGE,
  canvasConflictMessage,
  isCanvasConflictError,
  isCanvasSupersededError,
} from "./canvasConflict.ts";

// The two frozen pre-write conflict strings are both conflicts from the user's
// perspective: the head moved, or the revision the client expected no longer
// exists. The third string is the post-write supersession marker. The desktop
// `set_canvas` command produces all three client-side. The helpers must
// recognize each whether it arrives as an Error or a raw string (the Tauri IPC
// layer hands back either), and must not misfire on unrelated errors.

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
    "submit failed: conflict: canvas revision does not exist",
  );
  assert.equal(isCanvasConflictError(wrapped), true);
});

test("supersession marker is a post-write conflict, not a pre-write one", () => {
  const message = "conflict: canvas save was superseded by a concurrent write";
  // The post-write predicate matches it, as Error and as raw string.
  assert.equal(isCanvasSupersededError(new Error(message)), true);
  assert.equal(isCanvasSupersededError(message), true);
  // The pre-write predicate must NOT — the two carry different user guidance.
  assert.equal(isCanvasConflictError(message), false);
  // And a pre-write marker is not a supersession.
  assert.equal(
    isCanvasSupersededError("conflict: canvas changed since it was loaded"),
    false,
  );
});

test("canvasConflictMessage maps each marker to its distinct copy", () => {
  assert.equal(
    canvasConflictMessage("conflict: canvas changed since it was loaded"),
    CANVAS_CONFLICT_MESSAGE,
  );
  assert.equal(
    canvasConflictMessage("conflict: canvas revision does not exist"),
    CANVAS_CONFLICT_MESSAGE,
  );
  assert.equal(
    canvasConflictMessage(
      "conflict: canvas save was superseded by a concurrent write",
    ),
    CANVAS_SUPERSEDED_MESSAGE,
  );
  // Non-conflict errors fall through to null so callers show the raw message.
  assert.equal(canvasConflictMessage(new Error("relay unreachable")), null);
});

test("unrelated errors are not conflicts", () => {
  assert.equal(isCanvasConflictError(new Error("relay unreachable")), false);
  assert.equal(isCanvasConflictError("some other failure"), false);
  assert.equal(isCanvasConflictError(null), false);
  assert.equal(isCanvasConflictError(undefined), false);
  assert.equal(isCanvasSupersededError(null), false);
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
