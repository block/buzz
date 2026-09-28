/**
 * Persist New-message banner dismissals so View / Dismiss survive remount.
 * Device-level (not scoped) — banner is popover chrome, not per-agent data.
 */

import * as React from "react";

const STORAGE_KEY = "buzz-bestie-popover-new-message-dismissed.v1";
const MAX_IDS = 500;

type DismissedState = {
  messageIds: string[];
  threadRootIds: string[];
};

const listeners = new Set<() => void>();

let state: DismissedState = readStored();

function readStored(): DismissedState {
  if (typeof window === "undefined") {
    return { messageIds: [], threadRootIds: [] };
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { messageIds: [], threadRootIds: [] };
    const parsed = JSON.parse(raw) as Partial<DismissedState>;
    const messageIds = Array.isArray(parsed.messageIds)
      ? parsed.messageIds.filter(
          (id): id is string => typeof id === "string" && id.length > 0,
        )
      : [];
    const threadRootIds = Array.isArray(parsed.threadRootIds)
      ? parsed.threadRootIds.filter(
          (id): id is string => typeof id === "string" && id.length > 0,
        )
      : [];
    return {
      messageIds: messageIds.slice(-MAX_IDS),
      threadRootIds: threadRootIds.slice(-MAX_IDS),
    };
  } catch {
    return { messageIds: [], threadRootIds: [] };
  }
}

function writeStored(next: DismissedState): void {
  state = {
    messageIds: next.messageIds.slice(-MAX_IDS),
    threadRootIds: next.threadRootIds.slice(-MAX_IDS),
  };
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Best-effort persistence; in-memory still applies.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): DismissedState {
  return state;
}

function getServerSnapshot(): DismissedState {
  return { messageIds: [], threadRootIds: [] };
}

/** Current permanently dismissed New-message ids (message + thread root). */
export function useBestiePopoverNewMessageDismissed(): {
  messageIds: ReadonlySet<string>;
  threadRootIds: ReadonlySet<string>;
} {
  const snap = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    getServerSnapshot,
  );
  return React.useMemo(
    () => ({
      messageIds: new Set(snap.messageIds),
      threadRootIds: new Set(snap.threadRootIds),
    }),
    [snap],
  );
}

/** Permanently ack a banner target (and optionally more message ids in view). */
export function ackBestiePopoverNewMessages(input: {
  messageIds?: readonly string[];
  threadRootIds?: readonly string[];
}): void {
  const messageIds = [...state.messageIds];
  const threadRootIds = [...state.threadRootIds];
  let changed = false;
  for (const id of input.messageIds ?? []) {
    if (!id || messageIds.includes(id)) continue;
    messageIds.push(id);
    changed = true;
  }
  for (const id of input.threadRootIds ?? []) {
    if (!id || threadRootIds.includes(id)) continue;
    threadRootIds.push(id);
    changed = true;
  }
  if (!changed) return;
  writeStored({ messageIds, threadRootIds });
}

/** Test helper. */
export function __resetBestiePopoverNewMessageDismissedForTests(): void {
  state = { messageIds: [], threadRootIds: [] };
  try {
    window.localStorage?.removeItem(STORAGE_KEY);
  } catch {
    // ignore
  }
  for (const listener of listeners) listener();
}

export const BESTIE_POPOVER_NEW_MESSAGE_DISMISSED_STORAGE_KEY = STORAGE_KEY;
