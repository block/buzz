/**
 * Pure view-model helpers for the LiveKit call view. No React, no SDK imports —
 * `CallView` maps `livekit-client` enums onto the small string unions below and
 * feeds them here, so every branch is unit-testable without a renderer or a
 * live room (Phase 3 D-C test policy: pure helpers only).
 */

/** Soft "the session ends soon" warning fires at this elapsed time. */
export const CALL_SESSION_SOFT_WARN_MS = 3.5 * 60 * 60 * 1000;
/** HiveTalk's per-session ceiling. The hard cut is server-driven (a disconnect
 * with a server reason); this is only used for the countdown copy. */
export const CALL_SESSION_CAP_MS = 4 * 60 * 60 * 1000;

export type CallBannerTone = "info" | "warning" | "error";

export type CallBannerModel = {
  tone: CallBannerTone;
  title: string;
  detail?: string;
  /** Show a "Rejoin" action — set only for terminal states the user can retry. */
  showRejoin: boolean;
} | null;

/** Connection phases, collapsed from `livekit-client`'s `ConnectionState`
 * (`signalReconnecting` folds into `reconnecting`). */
export type CallConnectionPhase =
  | "connecting"
  | "connected"
  | "reconnecting"
  | "disconnected";

/** Disconnect buckets, collapsed from `livekit-client`'s `DisconnectReason`. */
export type CallDisconnectReason =
  | "user"
  | "server"
  | "duplicate"
  | "session-ended"
  | "unknown";

/**
 * Banner for a live (non-terminal) connection phase. `null` while connected —
 * the grid shows unobstructed. `disconnected` here means "dropped, retrying is
 * up to the user"; a clean user-initiated leave never reaches this helper
 * because `CallView` routes that straight to the leave navigation.
 */
export function connectionBannerModel(
  phase: CallConnectionPhase,
): CallBannerModel {
  switch (phase) {
    case "connecting":
      return {
        tone: "info",
        title: "Connecting to the room…",
        showRejoin: false,
      };
    case "reconnecting":
      return {
        tone: "warning",
        title: "Connection lost — reconnecting…",
        detail: "Your mic and camera resume automatically once you're back.",
        showRejoin: false,
      };
    case "disconnected":
      return {
        tone: "error",
        title: "Disconnected from the room",
        detail: "The connection dropped and couldn't recover on its own.",
        showRejoin: true,
      };
    case "connected":
      return null;
  }
}

/** Terminal banner for a resolved disconnect, keyed by reason bucket. Returns
 * `null` for a clean user-initiated leave (caller navigates away instead). */
export function disconnectBannerModel(
  reason: CallDisconnectReason,
): CallBannerModel {
  switch (reason) {
    case "user":
      return null;
    case "session-ended":
      return {
        tone: "info",
        title: "This meeting reached its time limit",
        detail:
          "Sessions are capped at 4 hours. Start a fresh room to continue.",
        showRejoin: true,
      };
    case "duplicate":
      return {
        tone: "warning",
        title: "You joined this room from another window",
        detail:
          "This tab was disconnected so the two don't fight over your mic.",
        showRejoin: true,
      };
    case "server":
      return {
        tone: "error",
        title: "The room closed",
        detail: "The host ended the meeting or the server shut the room down.",
        showRejoin: true,
      };
    case "unknown":
      return {
        tone: "error",
        title: "Disconnected from the room",
        showRejoin: true,
      };
  }
}

/** Soft time-cap warning; `null` until the soft threshold, then counts down to
 * the hard cap. */
export function sessionCapBannerModel(elapsedMs: number): CallBannerModel {
  if (!Number.isFinite(elapsedMs) || elapsedMs < CALL_SESSION_SOFT_WARN_MS) {
    return null;
  }
  const remainingMs = Math.max(0, CALL_SESSION_CAP_MS - elapsedMs);
  const remainingMin = Math.ceil(remainingMs / 60_000);
  return {
    tone: "warning",
    title:
      remainingMin > 0
        ? `This meeting ends in about ${remainingMin} min`
        : "This meeting is about to end",
    detail: "HiveTalk caps sessions at 4 hours.",
    showRejoin: false,
  };
}

/** `h:mm:ss` (or `m:ss` under an hour) for the elapsed-time pill. */
export function formatCallElapsed(ms: number): string {
  const total = Math.max(0, Math.floor((Number.isFinite(ms) ? ms : 0) / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${minutes}:${pad(seconds)}`;
}
