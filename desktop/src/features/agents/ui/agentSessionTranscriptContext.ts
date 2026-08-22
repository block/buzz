import * as React from "react";

/**
 * Presentation modes for the observer transcript.
 *
 * - `default`        — the polished activity feed (agent panel, thread panel).
 * - `compactPreview` — dense, single-column preview (profile panel cards).
 * - `conversation`   — full-cover focus-mode reading view: prompts as
 *                      right-aligned bubbles, agent messages as unboxed prose,
 *                      thoughts/plans as disclosures, lifecycle as quiet
 *                      dividers. Tool items render exactly as `default`.
 */
export type AgentSessionTranscriptVariant =
  | "default"
  | "compactPreview"
  | "conversation";

const AgentSessionTranscriptVariantContext =
  React.createContext<AgentSessionTranscriptVariant>("default");

export const AgentSessionTranscriptVariantProvider =
  AgentSessionTranscriptVariantContext.Provider;

export function useAgentSessionTranscriptVariant() {
  return React.useContext(AgentSessionTranscriptVariantContext);
}

/**
 * Per-render derivation the `conversation` variant needs but individual render
 * classes cannot see on their own: which item is currently streaming (the
 * trailing item of a live turn) and how long each thought ran before the turn
 * produced its next item. Derived once by the list from the display blocks —
 * see `buildConversationTurnMeta`.
 */
export type AgentSessionTranscriptTurnMeta = {
  /** Trailing item of a live turn, or null when nothing is streaming. */
  streamingItemId: string | null;
  /** Thought item id → seconds elapsed until the turn's next item. */
  thoughtDurationSecondsById: Record<string, number>;
};

export const EMPTY_TRANSCRIPT_TURN_META: AgentSessionTranscriptTurnMeta = {
  streamingItemId: null,
  thoughtDurationSecondsById: {},
};

const AgentSessionTranscriptTurnMetaContext =
  React.createContext<AgentSessionTranscriptTurnMeta>(
    EMPTY_TRANSCRIPT_TURN_META,
  );

export const AgentSessionTranscriptTurnMetaProvider =
  AgentSessionTranscriptTurnMetaContext.Provider;

export function useAgentSessionTranscriptTurnMeta() {
  return React.useContext(AgentSessionTranscriptTurnMetaContext);
}
