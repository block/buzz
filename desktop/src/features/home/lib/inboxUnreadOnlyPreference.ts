/** Persist Inbox "Show unread only" across navigations and restarts (#3669). */

export const INBOX_UNREAD_ONLY_STORAGE_KEY = "buzz.desktop.inbox-unread-only.v1";

export function readInboxUnreadOnlyPreference(
  storage: Pick<Storage, "getItem"> | null | undefined = typeof window !==
  "undefined"
    ? window.localStorage
    : null,
): boolean {
  if (!storage) return false;
  try {
    const raw = storage.getItem(INBOX_UNREAD_ONLY_STORAGE_KEY);
    if (raw === null) return false;
    return raw === "true" || raw === "1";
  } catch {
    return false;
  }
}

export function writeInboxUnreadOnlyPreference(
  value: boolean,
  storage: Pick<Storage, "setItem"> | null | undefined = typeof window !==
  "undefined"
    ? window.localStorage
    : null,
): void {
  if (!storage) return;
  try {
    storage.setItem(INBOX_UNREAD_ONLY_STORAGE_KEY, value ? "true" : "false");
  } catch {
    // Quota / private mode — preference is best-effort.
  }
}
