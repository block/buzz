import type {
  TranscriptTurnSegment,
  TranscriptDisplayBlock,
} from "./agentSessionTranscriptGrouping";
import { buildCompactToolSummary } from "./agentSessionToolSummary";
import type { TranscriptItem } from "./agentSessionTypes";

/** Reasoning. */
type WorkBlockThoughtItem = Extract<TranscriptItem, { type: "thought" }>;
/**
 * An interim agent note: an assistant message that is not the turn's answer.
 * The `role` intersection is what makes a *user* message unrepresentable as a
 * note, so the rail cannot present the reader's own prompt as agent progress.
 */
type WorkBlockNoteItem = Extract<TranscriptItem, { type: "message" }> & {
  role: "assistant";
};
/** A step the agent ran. */
type WorkBlockToolItem = Extract<TranscriptItem, { type: "tool" }>;

/**
 * The items a work block admits, as a closed union.
 *
 * This is the type-level half of `isWorkItem`: a block cannot hold a plan, a
 * lifecycle row or the turn's answer, so no downstream code has to consider
 * what those would look like on the rail. Adding a `TranscriptItem` variant to
 * this union is a deliberate act that forces a projection decision (see
 * `projectWorkBlockEntry`) rather than letting the new type inherit another
 * kind's presentation.
 */
export type WorkBlockItem =
  | WorkBlockThoughtItem
  | WorkBlockNoteItem
  | WorkBlockToolItem;

/**
 * One turn's work block: the thinking, tool steps and interim agent notes that
 * happened between the reader's prompt and the agent's answer, presented as a
 * single rail rather than as a row per item.
 */
export type TranscriptWorkBlock = {
  /**
   * Stable identity, derived from the FIRST item in the block. A block grows in
   * place as later steps stream in, so keying on the first item keeps the id
   * append-stable — anything derived from the last item would remount the block
   * on every append and throw away the reader's disclosure choice.
   */
  id: string;
  /** Steps in true arrival order. */
  items: WorkBlockItem[];
  /** Timestamp of the block's first step. */
  timestamp: string;
};

/**
 * A turn's segments as the `conversation` variant renders them: the shared
 * segment union plus the work block, which only this variant produces.
 */
export type TranscriptConversationSegment =
  | TranscriptTurnSegment
  | { kind: "work-block"; block: TranscriptWorkBlock };

/**
 * Which items are *work* — the material the block absorbs.
 *
 *  - **thoughts** — reasoning is a step on the rail, not a sibling disclosure.
 *  - **tool steps**, including failed ones, except message sends. A failure
 *    belongs to the work it happened in; the folded line reports it
 *    (`N steps · 1 failed`) so it is never hidden behind a neutral count.
 *    Message sends stay outside as chat bubbles: they are communication the
 *    reader should be able to read and open, not generic work telemetry.
 *  - **interim agent notes** — an assistant message that is not the turn's
 *    answer. Unlike a message-send tool, a note is narration within the turn,
 *    so it stays on the rail as progress.
 *
 * Everything else stays outside and keeps its own row. Plans stay a sibling
 * (Buzz's checklist is a first-class surface, not a step), and lifecycle rows —
 * permission gates, errors, status/compaction notices — are exactly the rows a
 * reader may need to act on or reinterpret the rest of the turn through, so
 * they never fold into a collapsed block.
 *
 * A type guard rather than a predicate, so the admitted set is checked once
 * here and every later stage receives `WorkBlockItem` — the reason the entry
 * projection can be exhaustive at all.
 */
function isWorkItem(
  item: TranscriptItem,
  finalAnswerId: string | null,
): item is WorkBlockItem {
  if (item.type === "thought") return true;
  if (item.type === "tool") {
    return buildCompactToolSummary(item).presentation !== "message";
  }
  return (
    item.type === "message" &&
    item.role === "assistant" &&
    item.id !== finalAnswerId
  );
}

/**
 * The turn's answer: its LAST assistant message.
 *
 * Deliberately positional rather than liveness-derived. While a turn streams,
 * the message currently arriving is already the presumptive answer, so the same
 * rule holds mid-turn and after it settles — the block does not reshuffle its
 * membership at the moment a turn completes.
 */
function findFinalAnswerId(segments: TranscriptTurnSegment[]): string | null {
  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (
      segment.kind === "item" &&
      segment.item.type === "message" &&
      segment.item.role === "assistant"
    ) {
      return segment.item.id;
    }
  }
  return null;
}

/**
 * The leaf items a segment contributes as candidate work, in reading order.
 *
 * Summary segments are expanded back to their leaf tool rows: the default
 * variant's "Read 3 files" summaries are a *different* answer to the same
 * grouping problem, and nesting them inside the block would give the reader two
 * collapsed layers to open before reaching a step. The block is the one
 * grouping in this variant.
 */
function segmentWorkCandidates(
  segment: TranscriptTurnSegment,
): TranscriptItem[] {
  if (segment.kind === "item") return [segment.item];
  if (segment.kind === "summary") return segment.summary.items;
  return [];
}

/**
 * The segment's candidates as admitted work items, or `null` when the segment
 * is not (entirely) work and therefore keeps its own row.
 *
 * All-or-nothing per segment, as before: a summary segment is one grouping
 * decision the shared code already made, so admitting half of it would emit its
 * remaining rows outside the block they belong to. Returning the narrowed array
 * rather than a boolean is what carries `WorkBlockItem` into the block — an
 * `.every()` guard cannot narrow the array it tested.
 */
function admittedWorkItems(
  segment: TranscriptTurnSegment,
  finalAnswerId: string | null,
): WorkBlockItem[] | null {
  const candidates = segmentWorkCandidates(segment);
  if (candidates.length === 0) return null;

  const admitted: WorkBlockItem[] = [];
  for (const item of candidates) {
    if (!isWorkItem(item, finalAnswerId)) return null;
    admitted.push(item);
  }
  return admitted;
}

/**
 * Collapse each maximal run of consecutive work items in a turn into one work
 * block.
 *
 * ## Why runs, and not "everything between the prompt and the answer"
 *
 * A turn is normally one block: prompt → (thinking, tools, notes) → answer, with
 * the plan checklist as a sibling. The span reading only differs when a
 * non-work row lands *inside* the work — a permission gate, an error, a
 * mid-turn plan update — and there the two readings disagree about order:
 * producing a single block would mean lifting that row out and re-emitting it
 * somewhere it did not happen.
 *
 * Splitting instead keeps every row where it occurred. That matters most for
 * exactly the rows this is about: a permission gate is a question asked at a
 * moment, and an error is the reason the steps after it look the way they do.
 * Moving either away from its position would cost the reader the thing that
 * makes it legible. So an interruption genuinely splits the work, and the seam
 * is the interruption itself.
 */
export function groupConversationWorkBlocks(
  segments: TranscriptTurnSegment[],
): TranscriptConversationSegment[] {
  const finalAnswerId = findFinalAnswerId(segments);
  const grouped: TranscriptConversationSegment[] = [];

  let pending: WorkBlockItem[] = [];
  const flush = () => {
    if (pending.length === 0) return;
    grouped.push({
      kind: "work-block",
      block: {
        id: `work-block:${pending[0].id}`,
        items: pending,
        timestamp: pending[0].timestamp,
      },
    });
    pending = [];
  };

  for (const segment of segments) {
    const work = admittedWorkItems(segment, finalAnswerId);

    if (work !== null) {
      pending.push(...work);
      continue;
    }

    flush();
    grouped.push(segment);
  }

  flush();
  return grouped;
}

/** Apply work-block grouping to every turn in a display block. */
export function conversationSegmentsForBlock(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
): TranscriptConversationSegment[] {
  return groupConversationWorkBlocks(block.segments);
}

/** A tool step's rail state. Only a tool step can be in flight or have failed. */
export type WorkBlockEntryState = "running" | "failed" | "settled";

/**
 * One projected rail row: what a row *is*, as a closed set.
 *
 *  - `thought` — reasoning.
 *  - `note` — an interim agent message: prose the agent addressed to the
 *    reader mid-turn, which is not the turn's answer. berd models this as a
 *    distinct `progress` entry and gives it the same speech-bubble glyph as a
 *    thought, because it reads as the agent talking, not as the agent acting.
 *  - `tool` — a step the agent ran.
 *
 * Projection happens ONCE per item, and everything downstream — the glyph, the
 * body, the folded line's counts — reads this rather than re-deriving it. Two
 * independent classifications of the same item is exactly how the headline and
 * the chain eligibility drifted apart on the abandoned tool-chain card, and
 * asking `item.type === ...` at each render site is what let a note fall
 * through to the tool branch and pick up a wrench.
 *
 * A discriminated union rather than a product of independent
 * `{ item, kind, state }` fields, so the model itself rules out the
 * combinations a product type leaves representable — and which every render
 * site would otherwise have to defend against:
 *
 *  - a `note` kind carrying a thought item (or vice versa), which is why the
 *    body switch previously had to re-check `item.type` and had a silent
 *    empty-string branch for the mismatch it could not otherwise handle;
 *  - prose that claims to be `running` or `failed`. Only a tool step has an
 *    outcome, so `state` is fixed to `settled` on the prose kinds — an edit
 *    that let a thought report a failure would not compile, rather than
 *    quietly adding to the folded line's `N failed`.
 */
export type WorkBlockEntry =
  | { kind: "thought"; item: WorkBlockThoughtItem; state: "settled" }
  | { kind: "note"; item: WorkBlockNoteItem; state: "settled" }
  | { kind: "tool"; item: WorkBlockToolItem; state: WorkBlockEntryState };

/**
 * A tool step's outcome.
 *
 * Order matters: a tool can carry a stale `isError` from a retry while the new
 * attempt executes, and reporting that as failed would fold a live block's
 * count to "N steps · 1 failed" while the work is still in flight.
 *
 * `executing`/`pending` only counts as *running* when a session actually owns
 * the step's turn. That status is written when the step starts and is never
 * revised if the agent dies first, so on its own it says "this step began", not
 * "this step is happening" — reopened history full of abandoned steps would
 * otherwise present as work in progress forever. See `liveTurnId`.
 *
 * An abandoned step is reported as `settled`, not as a new state: we do not
 * know that it failed, so it must not count toward the folded line's
 * `N failed`, and inventing a third outcome would put a marker on the rail for
 * something the reader cannot act on. It renders as the neutral step it is,
 * with whatever detail it managed to record.
 */
function toolEntryState(
  item: WorkBlockToolItem,
  liveTurnId: string | null,
): WorkBlockEntryState {
  if (item.status === "executing" || item.status === "pending") {
    // Compared rather than tested for truthiness: an item with no turn id
    // cannot be owned by a live turn, and `null === null` must not read as
    // ownership.
    return liveTurnId !== null && item.turnId === liveTurnId
      ? "running"
      : "settled";
  }
  if (item.isError || item.status === "failed") return "failed";
  return "settled";
}

/**
 * Project one admitted item into its rail entry.
 *
 * The switch is exhaustive over `WorkBlockItem` with no default: adding a
 * variant to that union without deciding what it looks like on the rail leaves
 * this function without an ending return, which is a type error. That is the
 * whole point of the closed union — a new item type must fail loudly here
 * instead of falling through to the tool branch and wearing a wrench.
 */
function projectWorkBlockEntry(
  item: WorkBlockItem,
  liveTurnId: string | null,
): WorkBlockEntry {
  switch (item.type) {
    case "thought":
      return { item, kind: "thought", state: "settled" };
    case "message":
      return { item, kind: "note", state: "settled" };
    case "tool":
      return { item, kind: "tool", state: toolEntryState(item, liveTurnId) };
  }
}

/**
 * Project a block's items into rail entries, in true arrival order.
 *
 * `liveTurnId` is required rather than optional: whether a step is running is
 * not a property of the step alone, and a default would let a caller that has
 * not thought about liveness get the old spins-forever behaviour silently.
 */
export function projectWorkBlockEntries(
  items: WorkBlockItem[],
  options: { liveTurnId: string | null },
): WorkBlockEntry[] {
  return items.map((item) => projectWorkBlockEntry(item, options.liveTurnId));
}

export type WorkBlockStatus = {
  /** Total steps on the rail. */
  count: number;
  /** Steps that failed. */
  failedCount: number;
  /**
   * Whether work is still happening: a step is pending/executing, or the turn
   * is live and its streaming item belongs to this block.
   */
  isActive: boolean;
};

/**
 * Aggregate state of a block, read off the SAME projection the rail renders.
 *
 * `isActive` is what decides live-vs-finished presentation, so it accepts both
 * evidence sources: an entry projected as `running`, and the list's
 * streaming-item hint. Either alone leaves a real gap — a thought streaming in
 * carries no tool status, and a tool left executing when the observer stream
 * drops would otherwise pin the block open forever if we trusted status alone.
 */
export function summarizeWorkBlock(
  entries: WorkBlockEntry[],
  options: { streamingItemId: string | null },
): WorkBlockStatus {
  let failedCount = 0;
  let isActive = false;

  for (const entry of entries) {
    if (entry.state === "failed") failedCount += 1;
    if (entry.state === "running") isActive = true;
    if (
      options.streamingItemId !== null &&
      entry.item.id === options.streamingItemId
    ) {
      isActive = true;
    }
  }

  return { count: entries.length, failedCount, isActive };
}

function pluralSteps(count: number) {
  return count === 1 ? "1 step" : `${count} steps`;
}

/**
 * The folded line for a finished block.
 *
 * berd's is a bare count. The failure clause is a deliberate departure: a count
 * alone is the one thing a reader cannot tell a clean run from a broken one by,
 * and a fold that hides a failure behind a neutral number invites them not to
 * open it.
 */
export function formatWorkBlockSummaryLabel(status: WorkBlockStatus): string {
  const steps = pluralSteps(status.count);
  if (status.failedCount === 0) return steps;
  return `${steps} · ${status.failedCount} failed`;
}

/** Label for the older steps tucked above the live window. */
export function formatPreviousStepsLabel(count: number): string {
  return count === 1 ? "1 previous step" : `${count} previous steps`;
}

/**
 * How many steps stay on the rail while work is in flight. The rest go behind
 * the "N previous steps" disclosure, so a long run does not push the answer off
 * screen while the reader is watching it arrive.
 */
export const WORK_BLOCK_LIVE_WINDOW_SIZE = 3;

export type WorkBlockWindow = {
  /** Entries rendered on the rail, in true order. */
  visibleEntries: WorkBlockEntry[];
  /** Older entries behind the disclosure, in true order. */
  hiddenEntries: WorkBlockEntry[];
};

/**
 * Chronological window over a live block: the last N steps in true arrival
 * order, with everything older behind the disclosure. A finished (or reader-
 * expanded) block shows every step, so windowing only applies while live.
 */
export function windowWorkBlockEntries(
  entries: WorkBlockEntry[],
  options: { isActive: boolean },
): WorkBlockWindow {
  if (!options.isActive || entries.length <= WORK_BLOCK_LIVE_WINDOW_SIZE) {
    return { hiddenEntries: [], visibleEntries: entries };
  }
  const splitAt = entries.length - WORK_BLOCK_LIVE_WINDOW_SIZE;
  return {
    hiddenEntries: entries.slice(0, splitAt),
    visibleEntries: entries.slice(splitAt),
  };
}
