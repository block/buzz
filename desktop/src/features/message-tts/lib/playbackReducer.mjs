/**
 * Pure state reducer for message read-aloud playback.
 *
 * The backend (`message_tts.rs`) emits `message-tts-state-changed` events;
 * this reducer folds them into the single-playback UI state the provider
 * exposes. Kept in `.mjs` (like `applyEditTagOverlay.mjs`) so the
 * TS-loader-less `node:test` runner imports the same source production uses;
 * typed access via the sibling `.d.mts`.
 *
 * Contract (mirrors the backend's generation guarantees):
 * - At most one message is "current" at a time.
 * - "preparing"/"playing" for message X makes X current, replacing anything.
 * - Terminal events ("completed" | "stopped" | "superseded") only clear the
 *   state when they name the CURRENT message — a stale terminal event for an
 *   already-replaced message must not clear the newer playback.
 * - "failed" for the current message keeps the id and carries the error so
 *   the row can surface it; "failed" for a non-current message becomes a
 *   detached error (toast-only), leaving current playback untouched.
 */

export const IDLE_STATE = Object.freeze({
  messageId: null,
  phase: "idle", // "idle" | "preparing" | "playing"
  truncated: false,
  error: null,
});

/**
 * Fold one backend playback event into the current UI state.
 * Returns `{ state, detachedError }`; `detachedError` is a failure for a
 * message that no longer owns playback (surface as a toast, not row state).
 */
export function applyPlaybackEvent(state, event) {
  const { message_id: messageId, status, truncated, error } = event;

  switch (status) {
    case "preparing":
    case "playing":
      return {
        state: {
          messageId,
          phase: status,
          truncated: Boolean(truncated),
          error: null,
        },
        detachedError: null,
      };
    case "completed":
    case "stopped":
    case "superseded":
      if (state.messageId === messageId) {
        return { state: IDLE_STATE, detachedError: null };
      }
      return { state, detachedError: null };
    case "failed": {
      const message = error || "playback failed";
      if (state.messageId === messageId || state.messageId === null) {
        return {
          state: { messageId, phase: "idle", truncated: false, error: message },
          detachedError: message,
        };
      }
      return { state, detachedError: message };
    }
    default:
      return { state, detachedError: null };
  }
}

/** Whether the play affordance for `messageId` should render as "playing". */
export function isMessagePlaying(state, messageId) {
  return (
    state.messageId === messageId &&
    (state.phase === "preparing" || state.phase === "playing")
  );
}
