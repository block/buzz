import * as React from "react";

export const SHOW_AGENT_THINKING_STORAGE_KEY =
  "buzz.messages.showAgentThinking";
/** On by default: per-message chrome is the primary day-to-day usage UX. */
export const DEFAULT_SHOW_AGENT_THINKING = true;

const listeners = new Set<() => void>();
let showAgentThinking = readStoredPreference();

export function parseShowAgentThinking(
  value: string | null | undefined,
): boolean {
  if (value === "false") return false;
  if (value === "true") return true;
  return DEFAULT_SHOW_AGENT_THINKING;
}

function readStoredPreference(): boolean {
  try {
    return parseShowAgentThinking(
      globalThis.localStorage?.getItem(SHOW_AGENT_THINKING_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_SHOW_AGENT_THINKING;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getShowAgentThinking(): boolean {
  return showAgentThinking;
}

export function setShowAgentThinking(value: boolean): void {
  if (value === showAgentThinking) return;
  showAgentThinking = value;
  try {
    globalThis.localStorage?.setItem(
      SHOW_AGENT_THINKING_STORAGE_KEY,
      String(value),
    );
  } catch {
    // Persistence is best-effort; the live preference still applies.
  }
  for (const listener of listeners) listener();
}

export function useShowAgentThinking(): boolean {
  return React.useSyncExternalStore(
    subscribe,
    getShowAgentThinking,
    () => DEFAULT_SHOW_AGENT_THINKING,
  );
}
