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
 * Two reject strings are both conflicts from the user's perspective:
 * - the head moved since load, and
 * - the revision the client expected no longer exists (e.g. it expected a head
 *   but the canvas was never created, or was replaced out from under it).
 *
 * Enforcement is client-side (no relay check today), so these strings are
 * produced by the desktop `set_canvas` command in
 * `desktop/src-tauri/src/commands/canvas.rs`. Keep them byte-identical there.
 */
const CANVAS_CONFLICT_MARKERS = [
  "conflict: canvas changed since it was loaded",
  "conflict: canvas revision does not exist",
] as const;

export const CANVAS_CONFLICT_MESSAGE =
  "This canvas changed since you loaded it — reload to see the latest, then reapply your edit.";

/**
 * Literal `expected-revision` value asserting "I expect no canvas exists yet".
 * Sent by the first save of a new canvas so a concurrent first creation is
 * detected as a conflict rather than silently overwritten. Matched by the
 * desktop `set_canvas` command; keep it byte-identical there.
 */
export const CANVAS_EXPECTED_REVISION_NONE = "none";

/**
 * True when `error` is the relay's optimistic-concurrency conflict — the head
 * moved or the expected revision no longer exists between the load and the
 * save. Accepts `Error` instances and raw strings so callers can pass whatever
 * the Tauri IPC layer hands them.
 */
export function isCanvasConflictError(error: unknown): boolean {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : null;
  if (message === null) {
    return false;
  }
  return CANVAS_CONFLICT_MARKERS.some((marker) => message.includes(marker));
}
