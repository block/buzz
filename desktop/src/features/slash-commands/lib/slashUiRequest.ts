import type { SlashCommandRunContext } from "./types";

export type SettledUiRequest<T = Record<string, never>> = T & {
  settle: (error?: Error) => void;
};

/**
 * Asks the component that owns the UI named `name` to act, and waits for it to
 * call `settle`. Rejects with `unavailable` when nothing owns `name`, or with
 * the error the owner settles with, so the command fails and keeps the draft.
 */
export function requestSlashCommandUiAndSettle<T extends object>(
  requestUi: SlashCommandRunContext["requestUi"],
  name: string,
  payload: T,
  unavailable: string,
): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const settle = (error?: Error) => (error ? reject(error) : resolve());
    if (!requestUi(name, { ...payload, settle })) {
      reject(new Error(unavailable));
    }
  });
}

/** The payload a `requestSlashCommandUiAndSettle` owner receives, or null. */
export function readSettledUiRequest<T extends object>(
  payload: unknown,
): SettledUiRequest<T> | null {
  if (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { settle?: unknown }).settle === "function"
  ) {
    return payload as SettledUiRequest<T>;
  }
  return null;
}

export function huddleSlashUiName(channelId: string): string {
  return `huddle:${channelId}`;
}

export function gifPickerSlashUiName(
  channelId: string,
  isThread: boolean,
): string {
  return `gif-picker:${channelId}:${isThread ? "thread" : "main"}`;
}
