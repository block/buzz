import * as React from "react";

/**
 * "Pull for changes" unread marks for pinned sites.
 *
 * Polling NEVER reloads a pin's webview — that would destroy the live page
 * (form input, scroll, logins) kept alive by the pin webview. A detected
 * change while the pin is not on screen only marks it unread here; viewing
 * the pin clears it. In-memory only (resets on app restart).
 */
const unread = new Set<string>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

export function markPinUnread(pinId: string): void {
  if (unread.has(pinId)) return;
  unread.add(pinId);
  emit();
}

export function clearPinUnread(pinId: string): void {
  if (!unread.delete(pinId)) return;
  emit();
}

export function isPinUnread(pinId: string): boolean {
  return unread.has(pinId);
}

export function subscribePinUnread(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePinUnread(pinId: string): boolean {
  return React.useSyncExternalStore(
    subscribePinUnread,
    () => unread.has(pinId),
    () => false,
  );
}

/** Test helper. */
export function resetPinUnread(): void {
  unread.clear();
  emit();
}
