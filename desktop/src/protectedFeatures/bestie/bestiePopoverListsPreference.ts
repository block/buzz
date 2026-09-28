import * as React from "react";

/**
 * Device-level preference: whether the Assistant popover Lists section
 * (Reminders / To-dos / Jobs / Coffee / Threads / Scratch) is collapsed while
 * the popover is open. Shared by no-agent and agent-assigned popovers.
 * Defaults collapsed. BestieProfileTrigger forces collapsed on each open so a
 * prior expand does not stick across sessions.
 */
const STORAGE_KEY = "buzz-bestie-popover-lists-collapsed.v1";

const listeners = new Set<() => void>();

let listsCollapsed = readStoredPreference();

function readStoredPreference(): boolean {
  if (typeof window === "undefined") {
    return true;
  }
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored == null ? true : stored === "1";
  } catch {
    return true;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): boolean {
  return listsCollapsed;
}

function getServerSnapshot(): boolean {
  return true;
}

/** Collapse or expand the popover Lists section. */
export function setBestiePopoverListsCollapsed(collapsed: boolean): void {
  listsCollapsed = collapsed;
  try {
    window.localStorage.setItem(STORAGE_KEY, collapsed ? "1" : "0");
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }
  for (const listener of listeners) {
    listener();
  }
}

/** Whether the Assistant popover Lists section is collapsed. */
export function useBestiePopoverListsCollapsed(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export const BESTIE_POPOVER_LISTS_COLLAPSED_STORAGE_KEY = STORAGE_KEY;
