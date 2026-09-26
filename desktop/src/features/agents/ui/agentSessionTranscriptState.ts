import type { TranscriptItem } from "./agentSessionTypes";

type PendingPermission = {
  itemId: string;
  optionNames: Map<string, string>;
};

export type TranscriptState = {
  items: TranscriptItem[];
  itemsById: Map<string, TranscriptItem>;
  activeMessageKey: Map<string, string>;
  sealedKeys: Set<string>;
  triggeringEventIdsByTurn: Map<string, string[]>;
  /**
   * Maps JSON-RPC request id → { itemId, optionNames }.
   * Populated when a `session/request_permission` request is ingested so the
   * matching response (which carries the same JSON-RPC id, no `method`) can
   * correlate and append the outcome to the lifecycle item.
   */
  pendingPermissions: Map<string, PendingPermission>;
  continuationSeq: number;
  latestSessionId: string | null;
};

export function createEmptyTranscriptState(): TranscriptState {
  return {
    items: [],
    itemsById: new Map(),
    activeMessageKey: new Map(),
    sealedKeys: new Set(),
    triggeringEventIdsByTurn: new Map(),
    pendingPermissions: new Map(),
    continuationSeq: 0,
    latestSessionId: null,
  };
}

/**
 * Mutable draft that collects changes during a single processTranscriptEvent
 * call. Replaces the previous pattern of nested closures capturing bare `let`
 * bindings — all mutation now targets this explicit object.
 */
export type TranscriptDraft = {
  items: TranscriptItem[];
  itemsById: Map<string, TranscriptItem>;
  activeMessageKey: Map<string, string>;
  sealedKeys: Set<string>;
  triggeringEventIdsByTurn: Map<string, string[]>;
  pendingPermissions: Map<string, PendingPermission>;
  continuationSeq: number;
  latestSessionId: string | null;
  changed: boolean;
};

export function createTranscriptDraft(state: TranscriptState): TranscriptDraft {
  return {
    items: state.items,
    itemsById: state.itemsById,
    activeMessageKey: state.activeMessageKey,
    sealedKeys: state.sealedKeys,
    triggeringEventIdsByTurn: state.triggeringEventIdsByTurn,
    pendingPermissions: state.pendingPermissions,
    continuationSeq: state.continuationSeq,
    latestSessionId: state.latestSessionId,
    changed: false,
  };
}
