/**
 * Device-local setting: how unaddressed channel messages reach agents.
 *
 * Label: "Unaddressed channel messages"
 * - Notify all channel agents  → "all-channel-agents" (default)
 * - Mentions only              → "mentions-only"
 *
 * Semantic storage key is versioned; not community/relay policy.
 */

import * as React from "react";

import type { UnaddressedChannelAgentMode } from "./contextualAgentConversationPolicy.ts";

/** Versioned device-local storage key (do not change without a migration). */
export const UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY =
  "buzz:unaddressed-channel-agent-mode:v1";

export const DEFAULT_UNADDRESSED_CHANNEL_AGENT_MODE: UnaddressedChannelAgentMode =
  "all-channel-agents";

const listeners = new Set<() => void>();

let mode: UnaddressedChannelAgentMode = readStoredMode();

export function parseUnaddressedChannelAgentMode(
  value: string | null | undefined,
): UnaddressedChannelAgentMode {
  return value === "mentions-only" || value === "all-channel-agents"
    ? value
    : DEFAULT_UNADDRESSED_CHANNEL_AGENT_MODE;
}

function readStoredMode(
  storage:
    | Pick<Storage, "getItem">
    | null
    | undefined = globalThis.localStorage,
): UnaddressedChannelAgentMode {
  try {
    return parseUnaddressedChannelAgentMode(
      storage?.getItem(UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_UNADDRESSED_CHANNEL_AGENT_MODE;
  }
}

export function readUnaddressedChannelAgentMode(
  storage:
    | Pick<Storage, "getItem">
    | null
    | undefined = globalThis.localStorage,
): UnaddressedChannelAgentMode {
  return readStoredMode(storage);
}

export function writeUnaddressedChannelAgentMode(
  next: UnaddressedChannelAgentMode,
  storage:
    | Pick<Storage, "setItem">
    | null
    | undefined = globalThis.localStorage,
): void {
  if (mode === next) {
    // Still persist in case storage was cleared while in-memory mode matched.
    try {
      storage?.setItem(UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY, next);
    } catch {
      // Best-effort.
    }
    return;
  }
  mode = next;
  try {
    storage?.setItem(UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY, next);
  } catch {
    // Best-effort persistence.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): UnaddressedChannelAgentMode {
  return mode;
}

function getServerSnapshot(): UnaddressedChannelAgentMode {
  return DEFAULT_UNADDRESSED_CHANNEL_AGENT_MODE;
}

/** Device-local unaddressed-channel agent mode for React consumers. */
export function useUnaddressedChannelAgentMode(): {
  mode: UnaddressedChannelAgentMode;
  setMode: (mode: UnaddressedChannelAgentMode) => void;
} {
  const current = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    getServerSnapshot,
  );
  return {
    mode: current,
    setMode: writeUnaddressedChannelAgentMode,
  };
}
