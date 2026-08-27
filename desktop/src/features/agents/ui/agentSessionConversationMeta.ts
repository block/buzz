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
 *
 * This walks the SHARED segment union deliberately. The conversation variant's
 * additive work-block transform runs later, at the list's render boundary
 * (`TranscriptDisplayBlockView`), so no `work-block` segment can reach here —
 * and the streaming tail must be computed from the true item order regardless
 * of how the variant later groups those items for presentation.
 */
function turnSegmentItems(segment: TranscriptTurnSegment): TranscriptItem[] {
  if (segment.kind === "prompt") return [segment.user];
  if (segment.kind === "setup") return [];
  if (segment.kind === "summary") return segment.summary.items;
  return [segment.item];
}

/**
 * Derive the `conversation` variant's live-turn hints from the display blocks.
 *
 * Two things the work block cannot see for itself:
 *
 *  - **`streamingItemId`** — the trailing leaf item of the final turn, and only
 *    when the agent's turn is actually live. The block reads this to decide
 *    whether it is still working: a thought or note streaming in carries no
 *    status of its own, so without this hint a block whose last step is prose
 *    would fold while the reader was still watching it arrive.
 *  - **`liveTurnId`** — which turn a session currently owns. A tool item keeps
 *    its `executing` status forever if the agent dies mid-step, so status alone
 *    cannot tell a running step from an abandoned one; the block needs to know
 *    whose turn is live to decide (see `AgentSessionTranscriptTurnMeta`).
 *
 * Returns the shared empty value for non-conversation variants so the default
 * and compactPreview paths allocate nothing and stay byte-identical.
 */
export function buildConversationTurnMeta(
  displayBlocks: TranscriptDisplayBlock[],
  options: {
    isTurnLive: boolean;
    items: TranscriptItem[];
    variant: AgentSessionTranscriptVariant;
  },
): AgentSessionTranscriptTurnMeta {
  if (options.variant !== "conversation" || !options.isTurnLive) {
    return EMPTY_TRANSCRIPT_TURN_META;
  }

  const lastBlock = displayBlocks[displayBlocks.length - 1];
  // The live turn is the newest turn in the ITEM stream, not the newest turn
  // that produced a block. See `latestTurnId` for why the distinction is
  // load-bearing. Read from the transcript rather than from the active-turn
  // store because the store's turn ids and the transcript's are populated by
  // different paths, and a mismatch would silently gate every step off.
  const liveTurnId = latestTurnId(options.items, displayBlocks);

  if (lastBlock?.kind === "single") {
    return {
      liveTurnId,
      streamingItemId: streamingIdForTail(lastBlock.item, liveTurnId),
    };
  }
  if (lastBlock?.kind !== "turn") {
    return { liveTurnId, streamingItemId: null };
  }

  const items = lastBlock.segments.flatMap(turnSegmentItems);
  const tail = items[items.length - 1];
  return {
    liveTurnId,
    streamingItemId: tail ? streamingIdForTail(tail, liveTurnId) : null,
  };
}

/**
 * The trailing item counts as streaming only if the LIVE turn owns it.
 *
 * Without this check the newest block's tail is reported as streaming no matter
 * whose turn it belongs to, so a finished turn's last step would hold that
 * turn's work block open while a *different* turn is the live one. That is the
 * same class of bug as an abandoned `executing` step: presenting settled history
 * as work in flight.
 */
function streamingIdForTail(
  tail: TranscriptItem,
  liveTurnId: string | null,
): string | null {
  // Compared rather than tested for truthiness, for the same reason as
  // `toolEntryState`: an item with no turn id is owned by nobody, and
  // `null === null` must not read as ownership.
  if (liveTurnId === null || tail.turnId !== liveTurnId) return null;
  return tail.id;
}

/**
 * The id of the newest turn the transcript has seen — including a turn that has
 * arrived but has nothing renderable in it yet.
 *
 * Read from the items rather than the blocks because a turn that has only
 * emitted setup lifecycle rows (`turn_started`, `session_resolved`) classifies
 * to zero segments and so produces NO block at all
 * (`agentSessionTranscriptGrouping`: `if (segments.length > 0)`). Walking the
 * blocks backwards for the last `turn` therefore skipped straight past the new
 * turn and returned the turn that had already ended — which then made its own
 * trailing item the streaming item, and a settled block visibly re-opened,
 * dropped to its last three steps and re-expanded for the whole gap between
 * `turn_started` and the new turn's first prompt or thought. That gap is real
 * observer-stream latency and happens on every turn, so this was not an edge
 * case.
 *
 * Items are in wire order, so the last one carrying a turn id names the newest
 * turn. Falls back to the last turn block for an item stream that carries no
 * turn ids at all.
 */
function latestTurnId(
  items: TranscriptItem[],
  displayBlocks: TranscriptDisplayBlock[],
): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const turnId = items[index]?.turnId;
    if (turnId) return turnId;
  }
  return lastTurnBlockId(displayBlocks);
}

/**
 * The id of the last turn block, ignoring anything that trails it.
 *
 * A lifecycle row (a compaction notice, a session boundary) can land after the
 * turn it belongs to and arrives as a `single` block, so the last block is not
 * always the live turn — but the last *turn* is.
 */
function lastTurnBlockId(
  displayBlocks: TranscriptDisplayBlock[],
): string | null {
  for (let index = displayBlocks.length - 1; index >= 0; index -= 1) {
    const block = displayBlocks[index];
    if (block.kind === "turn") return block.turnId;
  }
  return null;
}
