/**
 * Optimistic-concurrency conflict detection for the channel canvas.
 *
 * A conflict-checked save (`set_canvas` / restore) asserts the revision the
 * editor loaded via an `["expected-revision", <head event id | "none">]` tag.
 * The desktop Rust command reads the live head and compares locally before
 * publishing; when the head no longer matches what the client loaded it fails
 * with one of the frozen conflict strings below. Callers use this to render a
 * distinct "canvas changed — reload" state instead of a generic error.
 *
 * Two of the markers are *pre-write* rejections — the save never published
 * because the head moved (or the expected revision no longer exists) between
 * load and submit. The third is a *post-write* marker: the save DID publish,
 * but a concurrent write became the visible head before verification. Its
 * message is deliberately different — the edit is preserved in History, so the
 * user reloads and restores rather than re-typing a lost edit.
 *
 * Detection is client-side and best-effort: it catches a competing write that
 * is visible at check time. Preventing the race entirely requires relay-side
 * linearization (phase 2). These strings are produced by the desktop
 * `set_canvas` command in `desktop/src-tauri/src/commands/canvas.rs`; keep them
 * byte-identical there.
 */

/**
 * Post-write supersession marker: the save published, but a concurrent write is
 * now current. The edit is NOT lost — it is persisted in History. Kept separate
 * from `CANVAS_CONFLICT_MARKERS` because it carries a distinct user message.
 * Keep byte-identical to `CANVAS_SUPERSEDED` in `canvas.rs`.
 */
const CANVAS_SUPERSEDED_MARKER =
  "conflict: canvas save was superseded by a concurrent write";

const CANVAS_CONFLICT_MARKERS = [
  "conflict: canvas changed since it was loaded",
  "conflict: canvas revision does not exist",
] as const;

export const CANVAS_CONFLICT_MESSAGE =
  "This canvas changed since you loaded it — reload to see the latest, then reapply your edit.";

export const CANVAS_SUPERSEDED_MESSAGE =
  "A concurrent edit is now current. Your save was preserved in History — reload, then restore it if needed.";

/**
 * Literal `expected-revision` value asserting "I expect no canvas exists yet".
 * Sent by the first save of a new canvas so a concurrent first creation is
 * detected as a conflict rather than silently overwritten. Matched by the
 * desktop `set_canvas` command; keep it byte-identical there.
 */
export const CANVAS_EXPECTED_REVISION_NONE = "none";

/** Extract a comparable message from whatever the Tauri IPC layer hands back. */
function errorMessage(error: unknown): string | null {
  return error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : null;
}

/**
 * True when `error` is a *pre-write* precondition failure — the head moved or
 * the expected revision no longer exists between load and save, so the write
 * never published. Accepts `Error` instances and raw strings.
 */
export function isCanvasConflictError(error: unknown): boolean {
  const message = errorMessage(error);
  if (message === null) {
    return false;
  }
  return CANVAS_CONFLICT_MARKERS.some((marker) => message.includes(marker));
}

/**
 * True when `error` is the *post-write* supersession marker — the save
 * published but a concurrent write became current. Distinct from
 * {@link isCanvasConflictError} because the edit is preserved in History and the
 * user-facing guidance differs.
 */
export function isCanvasSupersededError(error: unknown): boolean {
  const message = errorMessage(error);
  if (message === null) {
    return false;
  }
  return message.includes(CANVAS_SUPERSEDED_MARKER);
}

/**
 * The user-facing message for a canvas save error, or `null` when the error is
 * not a canvas conflict (callers fall back to the raw error). Post-write
 * supersession takes precedence so its "preserved in History" guidance is never
 * masked by the generic conflict copy.
 */
export function canvasConflictMessage(error: unknown): string | null {
  if (isCanvasSupersededError(error)) {
    return CANVAS_SUPERSEDED_MESSAGE;
  }
  if (isCanvasConflictError(error)) {
    return CANVAS_CONFLICT_MESSAGE;
  }
  return null;
}
