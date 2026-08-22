import * as React from "react";

export type ComposerSubmitShortcut = "enter" | "mod-enter";

const STORAGE_KEY = "buzz:composer-submit-shortcut:v1";
const DEFAULT_SHORTCUT: ComposerSubmitShortcut = "enter";

const listeners = new Set<() => void>();
let currentShortcut = readShortcut();

function isComposerSubmitShortcut(
  value: unknown,
): value is ComposerSubmitShortcut {
  return value === "enter" || value === "mod-enter";
}

function readShortcut(): ComposerSubmitShortcut {
  if (typeof window === "undefined") return DEFAULT_SHORTCUT;
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return isComposerSubmitShortcut(stored) ? stored : DEFAULT_SHORTCUT;
  } catch {
    return DEFAULT_SHORTCUT;
  }
}

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getComposerSubmitShortcut(): ComposerSubmitShortcut {
  return currentShortcut;
}

export function setComposerSubmitShortcut(
  nextShortcut: ComposerSubmitShortcut,
): void {
  if (currentShortcut === nextShortcut) return;
  currentShortcut = nextShortcut;
  try {
    window.localStorage.setItem(STORAGE_KEY, nextShortcut);
  } catch {
    // Persistence is best-effort; the live setting still applies in memory.
  }
  emit();
}

export function useComposerSubmitShortcut(): ComposerSubmitShortcut {
  return React.useSyncExternalStore(
    subscribe,
    getComposerSubmitShortcut,
    () => DEFAULT_SHORTCUT,
  );
}
