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
 * trailing item of a live turn). Derived once by the list from the display
 * blocks — see `buildConversationTurnMeta`.
 */
export type AgentSessionTranscriptTurnMeta = {
  /** Trailing item of a live turn, or null when nothing is streaming. */
  streamingItemId: string | null;
  /**
   * The turn that is actually live, or null when no turn is.
   *
   * A tool item's `executing`/`pending` status is not evidence that work is
   * happening: an agent that dies after emitting a tool start leaves that
   * status on the item permanently, so reopened history still claims to be
   * mid-step. Whether a session owns that step is knowledge only the list has,
   * which is why it is published here rather than re-derived per row.
   *
   * A turn id rather than a boolean, because "some turn is live" is not the
   * question. An agent that crashed during turn 1 and is now working on turn 2
   * is live, yet turn 1's abandoned step is no more running than before — a
   * global flag would keep it spinning in exactly the case a restarted agent
   * makes common.
   */
  liveTurnId: string | null;
};

export const EMPTY_TRANSCRIPT_TURN_META: AgentSessionTranscriptTurnMeta = {
  liveTurnId: null,
  streamingItemId: null,
};

/**
 * Whether the surrounding subtree is a work block's rail.
 *
 * The rail's job is to show *what the agent did*, one muted row per step. Two
 * tool presentations disagree with that: a relay `messages send` classifies as
 * `renderClass: "message"` and therefore renders through
 * `CompactMessageSummary` — a 28px avatar, a bordered speech bubble, a
 * timestamp and a delivery-receipt button. That treatment is right in the
 * activity feed, where a posted message is a destination the reader may want to
 * open, and wrong on the rail, where the same markup reads as the agent
 * *replying* in the middle of its own work.
 *
 * This is the same failure the interim-note case avoids (see
 * `WorkBlockProseBody`), reached by a different route: there the item is an
 * assistant message, here it is a tool call that merely classifies as one. The
 * signal is explicit and additive rather than inferred from the transcript
 * variant, because `conversation` alone is not the condition — the same relay
 * step rendered outside a block in that variant should keep its bubble.
 *
 * Defaults to `false`, so `default` and `compactPreview` cannot observe it and
 * their markup stays byte-identical.
 */
const AgentSessionWorkBlockRailContext = React.createContext(false);

export const AgentSessionWorkBlockRailProvider =
  AgentSessionWorkBlockRailContext.Provider;

export function useIsInsideWorkBlockRail() {
  return React.useContext(AgentSessionWorkBlockRailContext);
}

const AgentSessionTranscriptTurnMetaContext =
  React.createContext<AgentSessionTranscriptTurnMeta>(
    EMPTY_TRANSCRIPT_TURN_META,
  );

export const AgentSessionTranscriptTurnMetaProvider =
  AgentSessionTranscriptTurnMetaContext.Provider;

export function useAgentSessionTranscriptTurnMeta() {
  return React.useContext(AgentSessionTranscriptTurnMetaContext);
}
