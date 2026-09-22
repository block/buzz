import * as React from "react";

export const INBOX_UNREAD_ONLY_KEY = "buzz.desktop.inbox-unread-only";

let memory: boolean | null = null;

function storage(): Storage | null {
  try {
    if (typeof window === "undefined") return null;
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readInboxUnreadOnlyPreference(): boolean {
  if (memory !== null) return memory;
  const raw = storage()?.getItem(INBOX_UNREAD_ONLY_KEY);
  memory = raw === "1";
  return memory;
}

export function writeInboxUnreadOnlyPreference(checked: boolean): void {
  memory = checked;
  const store = storage();
  if (!store) return;
  try {
    store.setItem(INBOX_UNREAD_ONLY_KEY, checked ? "1" : "0");
  } catch {
    // Quota / private-mode — in-memory still holds for this session.
  }
}

/** Test-only. */
export function resetInboxUnreadOnlyPreferenceForTests(): void {
  memory = null;
}

export function useInboxUnreadOnlyPreference(): [
  boolean,
  (checked: boolean) => void,
] {
  const [unreadOnly, setUnreadOnly] = React.useState(
    readInboxUnreadOnlyPreference,
  );
  const persist = React.useCallback((checked: boolean) => {
    writeInboxUnreadOnlyPreference(checked);
    setUnreadOnly(checked);
  }, []);
  return [unreadOnly, persist];
}
