import type { PresenceStatus } from "@/shared/api/types";

export const SLASH_UI_SET_PRESENCE = "set-presence";
export const SLASH_UI_OPEN_STATUS_DIALOG = "open-status-dialog";

/**
 * Payload of a `SLASH_UI_SET_PRESENCE` request. The presence session lives in
 * the app shell, so the handler stores its in-flight update in `result` for the
 * command to await and report.
 */
export type SetPresenceRequest = {
  status: PresenceStatus;
  result: Promise<void> | null;
};
