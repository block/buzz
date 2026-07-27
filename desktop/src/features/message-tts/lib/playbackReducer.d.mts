/**
 * Type declarations for `playbackReducer.mjs`. Runtime lives in `.mjs` so the
 * `node:test` runner can import it directly; this file gives TypeScript
 * callers a typed view.
 */

export type MessageTtsPhase = "idle" | "preparing" | "playing";

export type MessageTtsUiState = {
  messageId: string | null;
  phase: MessageTtsPhase;
  truncated: boolean;
  error: string | null;
};

export type MessageTtsBackendEvent = {
  message_id: string;
  status:
    | "preparing"
    | "playing"
    | "completed"
    | "stopped"
    | "superseded"
    | "failed";
  truncated: boolean;
  error: string | null;
};

export const IDLE_STATE: MessageTtsUiState;

export function applyPlaybackEvent(
  state: MessageTtsUiState,
  event: MessageTtsBackendEvent,
): { state: MessageTtsUiState; detachedError: string | null };

export function isMessagePlaying(
  state: MessageTtsUiState,
  messageId: string,
): boolean;
