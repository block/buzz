/**
 * Optimistic-concurrency conflict detection for the channel canvas.
 *
 * A conflict-checked save (`set_canvas` / restore) sends an
 * `["expected-revision", <head event id | "none">]` tag. The relay rejects the
 * write when the live head no longer matches what the client loaded, and the
 * Rust submit path surfaces that as an error whose message contains one of the
 * frozen relay strings below. Callers use this to render a distinct "canvas
 * changed — reload" state instead of a generic error.
 *
 * Two reject strings are both conflicts from the user's perspective:
 * - the head moved since load, and
 * - the revision the client expected no longer exists (e.g. it expected a head
 *   but the canvas was never created, or was replaced out from under it).
 * A third arises under contract v3's head-advancement guarantee: a write whose
 * precondition matches but which does not sort strictly ahead of the asserted
 * head (`created_at DESC, id ASC`) is rejected so an accepted tagged write is
 * always the new visible head.
 *
 * Contract: the relay reject strings are frozen (`crates/**`, Duncan's PR1). Do
 * not change these substrings without updating the relay in lockstep.
 */
const CANVAS_CONFLICT_MARKERS = [
  "conflict: canvas changed since it was loaded",
  "conflict: canvas revision does not exist",
  "conflict: canvas write does not supersede the current head",
] as const;

export const CANVAS_CONFLICT_MESSAGE =
  "This canvas changed since you loaded it — reload to see the latest, then reapply your edit.";

/**
 * Literal `expected-revision` value asserting "I expect no canvas exists yet".
 * Sent by the first save of a new canvas so a concurrent first creation is
 * rejected as a conflict rather than silently overwritten. Frozen contract
 * value (`crates/**`, Duncan's PR1).
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
