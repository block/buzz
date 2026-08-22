import type {
  AgentSessionTranscriptTurnMeta,
  AgentSessionTranscriptVariant,
} from "./agentSessionTranscriptContext";
import { EMPTY_TRANSCRIPT_TURN_META } from "./agentSessionTranscriptContext";
import type {
  TranscriptDisplayBlock,
  TranscriptTurnSegment,
} from "./agentSessionTranscriptGrouping";
import type { TranscriptItem } from "./agentSessionTypes";

/**
 * Flatten a turn's segments into the leaf items that the reader actually sees
 * in the order they appear. Prompt segments contribute their user message;
 * setup segments contribute nothing (they render as a quiet divider, not work);
 * summary segments contribute their collapsed tool items.
 */
function turnSegmentItems(segment: TranscriptTurnSegment): TranscriptItem[] {
  if (segment.kind === "prompt") return [segment.user];
  if (segment.kind === "setup") return [];
  if (segment.kind === "summary") return segment.summary.items;
  return [segment.item];
}

function elapsedSeconds(from: string, to: string): number | null {
  const start = Date.parse(from);
  const end = Date.parse(to);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  const seconds = Math.round((end - start) / 1_000);
  return seconds >= 0 ? seconds : null;
}

/**
 * Derive the `conversation` variant's streaming/duration hints from the display
 * blocks.
 *
 * Two facts the individual render classes cannot see on their own:
 *
 *  - **Which item is streaming.** Only the trailing leaf item of the final turn
 *    can be mid-stream, and only when the agent's turn is actually live. The
 *    thought disclosure auto-opens for exactly that item, so the reader watches
 *    reasoning arrive rather than clicking to find it.
 *  - **How long a thought ran.** A thought is finished once the turn produces
 *    ANY later leaf item — a tool call or plan update, not just the next
 *    assistant message. This is deliberate: reasoning recedes as soon as the
 *    agent *acts* on it, which is the moment the reader's attention should move
 *    to the action. Folding only on the next message would pin reasoning open
 *    across a long tool run, which is exactly the noise the focus view exists
 *    to remove. The gap between the thought's timestamp and that next item's
 *    timestamp is the "Thought for Ns" duration. Thoughts with no following
 *    item are still open, so they get no duration.
 *
 * Returns the shared empty value for non-conversation variants so the default
 * and compactPreview paths allocate nothing and stay byte-identical.
 */
export function buildConversationTurnMeta(
  displayBlocks: TranscriptDisplayBlock[],
  options: { isTurnLive: boolean; variant: AgentSessionTranscriptVariant },
): AgentSessionTranscriptTurnMeta {
  if (options.variant !== "conversation") {
    return EMPTY_TRANSCRIPT_TURN_META;
  }

  const thoughtDurationSecondsById: Record<string, number> = {};
  let streamingItemId: string | null = null;

  for (const block of displayBlocks) {
    if (block.kind !== "turn") continue;

    const items = block.segments.flatMap(turnSegmentItems);
    for (let index = 0; index < items.length; index++) {
      const item = items[index];
      if (item.type !== "thought") continue;
      const next = items[index + 1];
      if (!next) continue;
      const seconds = elapsedSeconds(item.timestamp, next.timestamp);
      if (seconds !== null) {
        thoughtDurationSecondsById[item.id] = seconds;
      }
    }
  }

  if (options.isTurnLive) {
    const lastBlock = displayBlocks[displayBlocks.length - 1];
    if (lastBlock?.kind === "turn") {
      const items = lastBlock.segments.flatMap(turnSegmentItems);
      streamingItemId = items[items.length - 1]?.id ?? null;
    } else if (lastBlock?.kind === "single") {
      streamingItemId = lastBlock.item.id;
    }
  }

  return { streamingItemId, thoughtDurationSecondsById };
}

/**
 * Disclosure label for a thought in the `conversation` variant.
 *
 * While the thought is the streaming tail of a live turn the reader is watching
 * reasoning arrive, so the label stays present tense. Once the turn has moved
 * on we know how long it ran and say so; a thought we cannot time (missing or
 * unparseable timestamps) still gets a stable past-tense label rather than a
 * misleading "0s".
 */
export function formatThoughtDisclosureLabel(options: {
  durationSeconds?: number;
  isStreaming: boolean;
}): string {
  if (options.isStreaming) {
    return "Thinking…";
  }
  const seconds = options.durationSeconds;
  if (seconds === undefined) {
    return "Thought";
  }
  if (seconds < 1) {
    return "Thought for a moment";
  }
  return `Thought for ${seconds}s`;
}
