import * as React from "react";

/** Device-level text sizing used across conversation surfaces. */
export type ConversationFontSize = "smaller" | "default" | "larger";

export const CONVERSATION_FONT_SIZE_STORAGE_KEY =
  "buzz.appearance.conversationFontSize";
export const DEFAULT_CONVERSATION_FONT_SIZE: ConversationFontSize = "default";

const listeners = new Set<() => void>();
let conversationFontSize: ConversationFontSize = DEFAULT_CONVERSATION_FONT_SIZE;

export function parseConversationFontSize(
  value: string | null | undefined,
): ConversationFontSize {
  return value === "smaller" || value === "default" || value === "larger"
    ? value
    : DEFAULT_CONVERSATION_FONT_SIZE;
}

function readStoredConversationFontSize(): ConversationFontSize {
  try {
    return parseConversationFontSize(
      globalThis.localStorage?.getItem(CONVERSATION_FONT_SIZE_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_CONVERSATION_FONT_SIZE;
  }
}

function applyConversationFontSize(size: ConversationFontSize): void {
  globalThis.document?.documentElement?.setAttribute(
    "data-conversation-font-size",
    size,
  );
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

/** Apply the persisted preference before React renders to avoid a layout jump. */
export function initializeConversationFontSizePreference(): void {
  const nextSize = readStoredConversationFontSize();
  const changed = nextSize !== conversationFontSize;
  conversationFontSize = nextSize;
  applyConversationFontSize(nextSize);
  if (changed) notifyListeners();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getConversationFontSize(): ConversationFontSize {
  return conversationFontSize;
}

export function setConversationFontSize(size: ConversationFontSize): void {
  conversationFontSize = size;
  applyConversationFontSize(size);
  try {
    globalThis.localStorage?.setItem(CONVERSATION_FONT_SIZE_STORAGE_KEY, size);
  } catch {
    // Persistence is best-effort; the live preference still applies.
  }
  notifyListeners();
}

/** Temporarily apply a size without changing the saved preference. */
export function previewConversationFontSize(
  size: ConversationFontSize | null,
): void {
  applyConversationFontSize(size ?? conversationFontSize);
}

export function useConversationFontSize(): ConversationFontSize {
  return React.useSyncExternalStore(
    subscribe,
    getConversationFontSize,
    () => DEFAULT_CONVERSATION_FONT_SIZE,
  );
}
