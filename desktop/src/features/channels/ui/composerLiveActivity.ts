import {
  getActivityHeadline,
  isMeaningfulItem,
  isSpineItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

/**
 * Lifecycle meta-frames that must never headline the pill. They pass the
 * spine filter (meaningful lifecycle items) but would surface as bare
 * "Usage" / "Commands" labels that read like phantom activity.
 */
const PILL_HEADLINE_EXCLUDED_SOURCES = new Set([
  "usage_update",
  "available_commands_update",
]);

export type ActivityPillHeadline = {
  /**
   * Transcript item id backing the headline. The pill keys its ticker swap
   * on this id, so a NEW action animates while streamed chunks that grow the
   * same item (message first line, thought text) update text in place.
   */
  id: string;
  label: string;
};

/**
 * Latest action headline for a working agent's composer pill.
 *
 * Tool items headline in the terse action format from getActivityHeadline —
 * verb + compact object ("Read foo.ts"), not "label · full preview" — so the
 * pill's narrow cap shows the informative part of the action.
 *
 * Channel-scoped, two-tier scan (spine items headline over metadata reads,
 * mirroring the session transcript's noise gate), newest wins. The headline
 * PERSISTS regardless of age — while a turn is in progress, the last real
 * action is more informative than a generic placeholder, so there is
 * deliberately no staleness decay (and no rotation through recent actions).
 * Returns null only when the transcript has nothing headline-able yet; the
 * pill then falls back to its generic "<name> is working…" label.
 */
export function deriveActivityPillLabel({
  channelId,
  transcript,
}: {
  channelId: string | null;
  transcript: readonly TranscriptItem[];
}): ActivityPillHeadline | null {
  const scoped = channelId
    ? transcript.filter((item) => item.channelId === channelId)
    : transcript;
  const passFilter = scoped.some(isSpineItem) ? isSpineItem : isMeaningfulItem;

  for (let index = scoped.length - 1; index >= 0; index -= 1) {
    const item = scoped[index];
    if (!item || !passFilter(item)) {
      continue;
    }
    if (
      item.type === "lifecycle" &&
      item.acpSource !== undefined &&
      PILL_HEADLINE_EXCLUDED_SOURCES.has(item.acpSource)
    ) {
      continue;
    }
    const headline = getActivityHeadline(item);
    if (!headline) {
      continue;
    }
    return { id: item.id, label: headline };
  }

  return null;
}

/**
 * Partition a channel's working agents into pill-rendered agents and the
 * combined typing-indicator group.
 *
 * Pill-worthiness is capability-based, not source-based: an agent renders as
 * a pill when there is an agent session worth hovering and opening — an
 * active observer turn, or a transcript with a headline-able item for this
 * channel left behind by a prior turn. Typing agents with nothing to show
 * fold into the combined typing group with the human typers.
 *
 * This is what keeps a pill stable across the turn-end gap: an agent whose
 * observer turn just completed but who is still typing keeps its pill (the
 * label swaps to "is typing…" in place) instead of demoting to the group,
 * and re-typing after a finished turn relights the pill rather than
 * mounting a group item. Only an agent's FIRST-ever activity in the channel
 * can start in the typing group.
 *
 * `getWorkingSource` / `getTranscript` are injected so the helper stays pure
 * and unit-testable; callers pass the working signal's and observer store's
 * cached readers.
 */
export function partitionComposerWorkingAgents({
  channelId,
  getTranscript,
  getWorkingSource,
  pubkeys,
}: {
  channelId: string | null;
  getTranscript: (pubkey: string) => readonly TranscriptItem[];
  getWorkingSource: (pubkey: string) => "observer" | "typing" | "none";
  pubkeys: readonly string[];
}): { pillPubkeys: string[]; typingGroupPubkeys: string[] } {
  const pillPubkeys: string[] = [];
  const typingGroupPubkeys: string[] = [];

  for (const pubkey of pubkeys) {
    if (getWorkingSource(pubkey) !== "typing") {
      pillPubkeys.push(pubkey);
      continue;
    }
    const headline = deriveActivityPillLabel({
      channelId,
      transcript: getTranscript(pubkey),
    });
    if (headline !== null) {
      pillPubkeys.push(pubkey);
    } else {
      typingGroupPubkeys.push(pubkey);
    }
  }

  return { pillPubkeys, typingGroupPubkeys };
}

/** Minimal working-state shape the ordering needs (see agentWorkingSignal). */
export type AgentWorkingChannelAnchor = {
  channelId: string;
  /** Desktop-clock anchor for when the agent started working there. */
  anchorAt: number;
};

/**
 * Working-agent pill order for the composer strip: agents ordered by when
 * they STARTED working in this channel (turn anchor), earliest first —
 * new agents append on the right. Deliberately not transcript recency:
 * a start anchor is stable for the whole turn, so pills hold their slot
 * while agents stream instead of shuffling on every event, and positional
 * motion is reserved for membership changes (an agent starting/finishing).
 *
 * Anchors are quantized to whole seconds — clock-offset refinement can
 * retroactively shift an anchor by the sub-second network delay it just
 * measured out, and such a shift must not reorder the strip. Ties (same
 * second, or no anchor for the requested scope) keep the incoming roster
 * order — the sort is stable, so they never reshuffle. Agents with no
 * anchor sort to the end.
 *
 * `getWorkingState` is injected so the helper stays pure and unit-testable;
 * callers pass the working signal's cached reader.
 */
export function deriveAgentWorkingOrder({
  channelId,
  getWorkingState,
  pubkeys,
}: {
  channelId: string | null;
  getWorkingState: (pubkey: string) => {
    channels: readonly AgentWorkingChannelAnchor[];
  };
  pubkeys: readonly string[];
}): string[] {
  const startSecond = new Map<string, number>();

  for (const pubkey of pubkeys) {
    let earliest: number | null = null;
    for (const channel of getWorkingState(pubkey).channels) {
      if (channelId !== null && channel.channelId !== channelId) {
        continue;
      }
      if (earliest === null || channel.anchorAt < earliest) {
        earliest = channel.anchorAt;
      }
    }
    if (earliest !== null) {
      startSecond.set(pubkey, Math.floor(earliest / 1000));
    }
  }

  return [...pubkeys].sort(
    (a, b) =>
      (startSecond.get(a) ?? Number.POSITIVE_INFINITY) -
      (startSecond.get(b) ?? Number.POSITIVE_INFINITY),
  );
}

/**
 * Latest activity timestamp (ms) for the composer preview's "Last live" pill.
 *
 * Reads the same sources the preview panel renders — the live transcript
 * window AND the channel-scoped archive — so the pill can never claim
 * "No activity yet" while archived rows are visible underneath. Falls back
 * to the active-turn anchor when a turn is running but no items exist yet.
 */
export function deriveLastLiveAt({
  activeTurns,
  archivedEvents,
  channelId,
  transcript,
}: {
  activeTurns: readonly { anchorAt: number; channelId: string }[];
  archivedEvents: readonly { timestamp: string }[];
  channelId: string | null;
  transcript: readonly {
    channelId?: string | null;
    timestamp: string;
  }[];
}): number | null {
  let latest: number | null = null;
  const record = (timestamp: number) => {
    if (latest === null || timestamp > latest) {
      latest = timestamp;
    }
  };

  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    const item = transcript[index];
    if (!item || (channelId && item.channelId !== channelId)) {
      continue;
    }
    const millis = Date.parse(item.timestamp);
    if (!Number.isNaN(millis)) {
      record(millis);
      break;
    }
  }

  // Archived events are already channel-scoped by the store, sorted ascending.
  for (let index = archivedEvents.length - 1; index >= 0; index -= 1) {
    const event = archivedEvents[index];
    if (!event) {
      continue;
    }
    const millis = Date.parse(event.timestamp);
    if (!Number.isNaN(millis)) {
      record(millis);
      break;
    }
  }

  const channelTurn = channelId
    ? activeTurns.find((turn) => turn.channelId === channelId)
    : activeTurns[0];
  if (channelTurn) {
    record(channelTurn.anchorAt);
  }

  return latest;
}
