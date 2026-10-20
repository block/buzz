import * as React from "react";

/** Device-level spacing used across conversation surfaces. */
export type ConversationDensity = "compact" | "comfortable" | "spacious";

export const CONVERSATION_DENSITY_STORAGE_KEY =
  "buzz.appearance.conversationDensity";
export const DEFAULT_CONVERSATION_DENSITY: ConversationDensity = "comfortable";

const listeners = new Set<() => void>();
let conversationDensity: ConversationDensity = DEFAULT_CONVERSATION_DENSITY;
let listeningForStorageChanges = false;

export function parseConversationDensity(
  value: string | null | undefined,
): ConversationDensity {
  return value === "compact" || value === "comfortable" || value === "spacious"
    ? value
    : DEFAULT_CONVERSATION_DENSITY;
}

function readStoredConversationDensity(): ConversationDensity {
  try {
    return parseConversationDensity(
      globalThis.localStorage?.getItem(CONVERSATION_DENSITY_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_CONVERSATION_DENSITY;
  }
}

function applyConversationDensity(density: ConversationDensity): void {
  globalThis.document?.documentElement?.setAttribute(
    "data-conversation-density",
    density,
  );
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

function applyStoredConversationDensity(): void {
  const nextDensity = readStoredConversationDensity();
  const changed = nextDensity !== conversationDensity;
  conversationDensity = nextDensity;
  applyConversationDensity(nextDensity);
  if (changed) notifyListeners();
}

function listenForStorageChanges(): void {
  if (listeningForStorageChanges || !globalThis.window?.addEventListener)
    return;
  globalThis.window.addEventListener("storage", (event) => {
    if (event.key === CONVERSATION_DENSITY_STORAGE_KEY || event.key === null) {
      applyStoredConversationDensity();
    }
  });
  listeningForStorageChanges = true;
}

/** Apply the persisted preference before React renders to avoid a layout jump. */
export function initializeConversationDensityPreference(): void {
  applyStoredConversationDensity();
  listenForStorageChanges();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getConversationDensity(): ConversationDensity {
  return conversationDensity;
}

export function setConversationDensity(density: ConversationDensity): void {
  conversationDensity = density;
  applyConversationDensity(density);
  try {
    globalThis.localStorage?.setItem(CONVERSATION_DENSITY_STORAGE_KEY, density);
  } catch {
    // Persistence is best-effort; the live preference still applies.
  }
  notifyListeners();
}

/** Temporarily apply a density without changing the saved preference. */
export function previewConversationDensity(
  density: ConversationDensity | null,
): void {
  applyConversationDensity(density ?? conversationDensity);
}

export function useConversationDensity(): ConversationDensity {
  return React.useSyncExternalStore(
    subscribe,
    getConversationDensity,
    () => DEFAULT_CONVERSATION_DENSITY,
  );
}
