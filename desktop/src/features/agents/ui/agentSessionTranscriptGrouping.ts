import { buildTranscriptState } from "./agentSessionTranscript";
import type { ObserverEvent, TranscriptItem } from "./agentSessionTypes";
import { classifyToolItem } from "./agentSessionToolClassifier";

export type TranscriptTurnSegment =
  | { kind: "item"; item: TranscriptItem }
  | { kind: "summary"; summary: TranscriptToolRunSummary }
  | { kind: "setup"; items: Extract<TranscriptItem, { type: "lifecycle" }>[] }
  | {
      kind: "prompt";
      user: Extract<TranscriptItem, { type: "message" }>;
      context: Extract<TranscriptItem, { type: "metadata" }> | null;
      setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
    };

export type TranscriptDisplayBlock =
  | { kind: "single"; item: TranscriptItem }
  | { kind: "turn"; turnId: string; segments: TranscriptTurnSegment[] }
  | {
      /**
       * Session boundary divider injected between consecutive session runs.
       * `sessionId` is the id of the session that FOLLOWS the divider (the
       * newer session in reading order).
       *
       * `labelState` encodes three distinct states:
       *  - `"current"`     — newest-visible session AND matches the live relay
       *                      session id. Agent is actively running this session.
       *  - `"most-recent"` — newest-visible session but no live session match
       *                      (archived-only view or session ended). This is the
       *                      most recently observed session — not current context.
       *  - `"earlier"`     — an older session, not newest-visible.
       */
      kind: "session-boundary";
      sessionId: string;
      sessionStartTimestamp: string;
      labelState: "current" | "most-recent" | "earlier";
      /**
       * Zero-based position of this boundary in the run array (i.e. the index
       * of the run it precedes, which is always ≥ 1). Used as a tiebreaker in
       * the React list key so that two non-contiguous runs sharing the same
       * sessionId produce DISTINCT keys and React never duplicates/omits them.
       *
       * @deprecated Use `firstItemId` for stable React keys instead. `runIndex`
       * shifts when older sessions are prepended before existing runs, causing
       * key churn and unnecessary remounting of unchanged boundary nodes.
       * Kept for callers that use position for non-key purposes.
       */
      runIndex: number;
      /**
       * The `id` of the first `TranscriptItem` in the run that follows this
       * boundary. Stable across prepend: older runs inserted before this run
       * do not change the first item of this run, so this field never churns
       * on archive-page loads.
       *
       * Use this field (not `runIndex`) as the React list key component for
       * session-boundary rows.
       */
      firstItemId: string;
    };

export type TranscriptToolRunChildSegment =
  | { kind: "item"; item: TranscriptItem }
  | { kind: "summary"; summary: TranscriptToolRunSummary };

export type TranscriptToolRunSummary = {
  id: string;
  label: string;
  count: number;
  /** Flat leaf tool items in original order (nested summaries expanded). */
  items: TranscriptItem[];
  renderClass: TranscriptItem["renderClass"] | null;
  /**
   * "same-kind" summaries collapse runs sharing one semantic groupKey and get
   * specific labels ("Read 3 files"). "mixed" summaries collapse broader
   * bursts of routine tool work ("Ran 9 tool calls") and may contain nested
   * same-kind summaries as children.
   */
  variant: "same-kind" | "mixed";
  /**
   * Child segments in original order for mixed bursts — raw tool rows plus
   * any same-kind summaries that joined the burst. Absent on same-kind
   * summaries, whose children are just `items`.
   */
  segments?: TranscriptToolRunChildSegment[];
  timestamp: string;
};

function isUserPrompt(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "message" }> {
  return (
    item.type === "message" &&
    item.role === "user" &&
    item.acpSource === "session/prompt:user"
  );
}

function isSteerPrompt(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "message" }> {
  return (
    item.type === "message" &&
    item.role === "user" &&
    item.acpSource === "session/steer:user"
  );
}

function isPromptContext(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "metadata" }> {
  return (
    item.type === "metadata" && item.acpSource === "session/prompt:context"
  );
}

function isSteerContext(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "metadata" }> {
  return item.type === "metadata" && item.acpSource === "session/steer:context";
}

function isSystemPrompt(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "metadata" }> {
  return item.type === "metadata" && item.acpSource === "session/new";
}

function isSetupLifecycle(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "lifecycle" }> {
  return (
    item.type === "lifecycle" &&
    (item.acpSource === "turn_started" || item.acpSource === "session_resolved")
  );
}

type TurnBucket = {
  turnId: string;
  items: TranscriptItem[];
};

function classifyTurnItems(items: TranscriptItem[]): TranscriptTurnSegment[] {
  const userPrompt = items.find(isUserPrompt) ?? null;
  const setupLifecycle = items.filter(isSetupLifecycle);
  const promptContext = items.find(isPromptContext) ?? null;
  // Steer context rides behind the steer message bubble's checks-icon dialog
  // (same ingress treatment as session/prompt context) instead of rendering
  // as a standalone "Prompt context" metadata row.
  const steerContexts = items.filter(isSteerContext);
  const consumed = new Set<TranscriptItem>();

  if (userPrompt) consumed.add(userPrompt);
  for (const item of setupLifecycle) consumed.add(item);
  if (promptContext) consumed.add(promptContext);
  for (const item of steerContexts) consumed.add(item);

  const activity = items.filter((item) => !consumed.has(item));
  const pendingSteerContexts = [...steerContexts];

  const activitySegments: TranscriptTurnSegment[] = activity.map((item) => {
    if (isSteerPrompt(item)) {
      return {
        kind: "prompt",
        user: item,
        context: pendingSteerContexts.shift() ?? null,
        setup: [],
      };
    }
    return { kind: "item", item };
  });

  // Steer context without a matched steer message keeps its standalone row so
  // the metadata is never silently dropped.
  for (const orphan of pendingSteerContexts) {
    activitySegments.push({ kind: "item", item: orphan });
  }

  if (!userPrompt) {
    return groupToolSegments(activitySegments);
  }

  const segments: TranscriptTurnSegment[] = [
    {
      kind: "prompt",
      user: userPrompt,
      context: promptContext,
      setup: setupLifecycle,
    },
    ...activitySegments,
  ];

  return groupToolSegments(segments);
}

/**
 * Two-pass tool grouping:
 * 1. Same-kind runs collapse into summaries with specific labels
 *    ("Read 3 files", "Edited 2 files").
 * 2. Leftover adjacent eligible tool rows of differing kinds collapse into a
 *    mixed fallback summary ("Ran 5 tool calls").
 *
 * Messages, errors, permissions, and status/lifecycle rows never join either
 * pass, so intervention points stay visible.
 */
function groupToolSegments(
  segments: TranscriptTurnSegment[],
): TranscriptTurnSegment[] {
  return groupMixedToolRuns(groupSameKindSegments(segments));
}

function groupSameKindSegments(
  segments: TranscriptTurnSegment[],
): TranscriptTurnSegment[] {
  const grouped: TranscriptTurnSegment[] = [];
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i];
    if (segment.kind !== "item") {
      grouped.push(segment);
      continue;
    }
    const key = sameKindKey(segment.item);
    if (!key) {
      grouped.push(segment);
      continue;
    }
    const run = [segment.item];
    let j = i + 1;
    while (j < segments.length) {
      const next = segments[j];
      if (next.kind !== "item" || sameKindKey(next.item) !== key) break;
      run.push(next.item);
      j += 1;
    }
    if (run.length >= minimumSummaryRunLength(run[0])) {
      grouped.push({
        kind: "summary",
        summary: {
          id: `summary:${key}:${run[0].id}`,
          label: sameKindLabel(run[0], run.length),
          count: run.length,
          items: run,
          renderClass: getRenderClass(run[0]),
          variant: "same-kind",
          timestamp: run[0].timestamp,
        },
      });
      i = j - 1;
    } else {
      grouped.push(...run.map((item) => ({ kind: "item" as const, item })));
      i = j - 1;
    }
  }
  return grouped;
}

const MIXED_RUN_MINIMUM_SEGMENTS = 2;

/**
 * Burst pass: collapse an interleave-tolerant run of routine tool work into
 * one "Ran N tool calls" summary. Both leftover raw eligible tool rows and
 * same-kind summaries produced by the first pass participate, so alternating
 * patterns like search → read-summary → search → read-summary collapse into a
 * single supervision row whose children are the original segments in order.
 * Messages, permissions, errors/failed tools, and status/suppressed rows
 * break bursts, so intervention points stay visible.
 */
function groupMixedToolRuns(
  segments: TranscriptTurnSegment[],
): TranscriptTurnSegment[] {
  const grouped: TranscriptTurnSegment[] = [];
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i];
    if (!isBurstParticipant(segment)) {
      grouped.push(segment);
      continue;
    }
    const run: TranscriptToolRunChildSegment[] = [segment];
    let j = i + 1;
    while (j < segments.length) {
      const next = segments[j];
      if (!isBurstParticipant(next)) break;
      run.push(next);
      j += 1;
    }
    if (run.length >= MIXED_RUN_MINIMUM_SEGMENTS) {
      const items = run.flatMap((child) =>
        child.kind === "item" ? [child.item] : child.summary.items,
      );
      // Mixed bursts are already the visual summary. Expanding nested
      // same-kind summaries here creates redundant rows like
      // "Ran 16 tool calls" → "Ran 12 commands". Keep same-kind summaries as
      // grouping inputs, but flatten the mixed summary's visible children back
      // to leaf tool rows.
      const childSegments = items.map((item) => ({
        kind: "item" as const,
        item,
      }));
      grouped.push({
        kind: "summary",
        summary: {
          id: `summary:mixed:${items[0].id}`,
          label: `Ran ${items.length} tool calls`,
          count: items.length,
          items,
          renderClass: null,
          variant: "mixed",
          segments: childSegments,
          timestamp: items[0].timestamp,
        },
      });
    } else {
      grouped.push(...run);
    }
    i = j - 1;
  }
  return grouped;
}

/**
 * Burst participants are raw eligible tool rows and same-kind summaries
 * (already-collapsed routine tool work). Mixed summaries never re-enter.
 */
function isBurstParticipant(
  segment: TranscriptTurnSegment,
): segment is TranscriptToolRunChildSegment {
  if (segment.kind === "item") {
    return isGroupingEligible(segment.item);
  }
  return segment.kind === "summary" && segment.summary.variant === "same-kind";
}

const GROUPING_ELIGIBLE_RENDER_CLASSES = new Set<
  NonNullable<TranscriptItem["renderClass"]>
>([
  "file-read",
  "skill-read",
  "shell",
  "relay-op",
  "file-edit",
  "image",
  "plan",
  "generic",
]);

/**
 * Shared eligibility for both grouping passes. Failed tools (isError or
 * reclassified renderClass "error"), messages, permissions, status, and
 * suppressed rows are never grouped and break runs, so intervention points
 * stay visible.
 */
function isGroupingEligible(item: TranscriptItem): boolean {
  if (item.type !== "tool" || item.isError) return false;
  const renderClass = getRenderClass(item);
  return (
    renderClass != null && GROUPING_ELIGIBLE_RENDER_CLASSES.has(renderClass)
  );
}

function sameKindKey(item: TranscriptItem): string | null {
  if (!isGroupingEligible(item) || item.type !== "tool") return null;
  const descriptor = item.descriptor ?? classifyToolItem(item);
  return descriptor.groupKey ?? getRenderClass(item);
}

function sameKindLabel(item: TranscriptItem, count: number): string {
  if (item.type !== "tool") return `${count} items`;
  const descriptor = item.descriptor ?? classifyToolItem(item);
  const renderClass = getRenderClass(item);
  const label = descriptor.label;
  if (renderClass === "file-edit") {
    return `Edited ${count} file${count === 1 ? "" : "s"}`;
  }
  if (renderClass === "file-read") return `Read ${count} files`;
  if (renderClass === "skill-read") {
    return `Read ${count} skill${count === 1 ? "" : "s"}`;
  }
  if (renderClass === "shell") return `Ran ${count} commands`;
  if (renderClass === "relay-op") return `Ran ${count} Buzz relay ops`;
  return `${label} ×${count}`;
}

function minimumSummaryRunLength(item: TranscriptItem): number {
  return getRenderClass(item) === "file-edit" ? 2 : 3;
}

function getRenderClass(item: TranscriptItem) {
  if (item.type !== "tool") return item.renderClass;
  const descriptor = item.descriptor ?? classifyToolItem(item);
  return item.renderClass ?? descriptor.renderClass;
}

/**
 * Split a flat, time-ordered array of TranscriptItems into contiguous session
 * runs keyed by `sessionId`.
 *
 * **Null-session handling**: the real first-turn wire sequence is
 * `turn_started(null) → session/new(null) → session_resolved(sess-X) → …`.
 * Pre-resolution items arrive with `sessionId: null` before any session has
 * been assigned. We defer those leading null-session items and **prepend them
 * to the first run that has a non-null sessionId**, so they stay in the same
 * session run as the turn they belong to.
 *
 * **Restart / session-boundary ordering**: on a restart the normalizer stamps
 * `session/new` with `latestSessionId` (the OLD session's id) because the new
 * session hasn't resolved yet. So the item arrives with `sessionId = "sess-1"`
 * (stale), not null. Under the plain grouping rule it lands in the prior run,
 * causing System Prompt to render ABOVE the session-boundary divider.
 *
 * Fix: `session/new` markers (`isSystemPrompt`) are session-START signals.
 * When one arrives and a prior run already exists, park it (and any null-session
 * items that follow) in `pendingNewRunBuffer`. Multiple markers may arrive
 * before the new session resolves (rapid restart loop) — all are accumulated
 * in order. When the next distinct non-null sessionId resolves, flush the
 * buffer to the HEAD of that new run — placing all System Prompt cards after
 * the boundary. If no new session ever resolves after the marker(s) (stream
 * ends mid-restart), flush back into the current run so nothing is dropped.
 *
 * Mid-stream null-session items (after at least one session has resolved, and
 * no pending-new-run buffer is open) are attributed to the most recently seen
 * session run — handling gap frames that arrive after resolution.
 *
 * Only if the entire stream is null-session (no session ever resolves) do the
 * deferred items form a single fallback run keyed `"unknown"`.
 *
 * A new run begins whenever the sessionId changes to a distinct non-null value.
 */
function splitIntoSessionRuns(
  items: TranscriptItem[],
): Array<{ sessionId: string; items: TranscriptItem[] }> {
  const runs: Array<{ sessionId: string; items: TranscriptItem[] }> = [];
  let currentRun: { sessionId: string; items: TranscriptItem[] } | null = null;
  // Buffer for items that arrive before any session has resolved.
  const preSessionBuffer: TranscriptItem[] = [];
  // Buffer for session/new marker(s) (and any null-session items trailing them)
  // that must be re-anchored to the NEXT resolved session (restart scenario).
  let pendingNewRunBuffer: TranscriptItem[] | null = null;

  for (const item of items) {
    if (item.sessionId === null || item.sessionId === undefined) {
      if (currentRun === null) {
        // No session resolved yet — defer into the pre-session buffer.
        preSessionBuffer.push(item);
      } else if (pendingNewRunBuffer !== null) {
        // Pending buffer already open — park trailing null-session items here.
        pendingNewRunBuffer.push(item);
      } else {
        // Ordinary mid-stream null-session item — attribute to current run.
        currentRun.items.push(item);
      }
      continue;
    }

    // item.sessionId is non-null from here.

    // A session/new marker after a prior run is a restart signal: park it in
    // the pending buffer so it re-anchors to the next distinct session run.
    // Multiple session/new markers may arrive before the new session resolves
    // (e.g. a rapid restart loop) — accumulate all of them rather than
    // reinitializing, so no marker is silently dropped.
    if (isSystemPrompt(item) && currentRun !== null) {
      if (pendingNewRunBuffer !== null) {
        pendingNewRunBuffer.push(item);
      } else {
        pendingNewRunBuffer = [item];
      }
      continue;
    }

    if (!currentRun || item.sessionId !== currentRun.sessionId) {
      const newRun: { sessionId: string; items: TranscriptItem[] } = {
        sessionId: item.sessionId,
        items: [],
      };
      if (currentRun === null && preSessionBuffer.length > 0) {
        // First resolved session: prepend buffered pre-resolution items.
        newRun.items.push(...preSessionBuffer);
        preSessionBuffer.length = 0;
      } else if (pendingNewRunBuffer !== null) {
        // New distinct session after a session/new marker: the buffered items
        // open this run (placed before its first regular item).
        newRun.items.push(...pendingNewRunBuffer);
        pendingNewRunBuffer = null;
      }
      currentRun = newRun;
      runs.push(currentRun);
    } else if (pendingNewRunBuffer !== null) {
      // Same sessionId resolved again — the session/new didn't precede a new
      // session. Flush the buffer into the current run so nothing is dropped.
      currentRun.items.push(...pendingNewRunBuffer);
      pendingNewRunBuffer = null;
    }
    currentRun.items.push(item);
  }

  // Stream ended with an open pending buffer (session/new seen, no new session
  // resolved after it) — flush into the current run so nothing is dropped.
  if (pendingNewRunBuffer !== null && currentRun !== null) {
    currentRun.items.push(...pendingNewRunBuffer);
  }

  // Entire stream was null-session (no session ever resolved): emit as one run.
  if (currentRun === null && preSessionBuffer.length > 0) {
    runs.push({ sessionId: "unknown", items: preSessionBuffer });
  }

  return runs;
}

/**
 * Build presentation-only display blocks from normalized transcript items.
 * Raw observer order is preserved in the source items; this only reorders
 * within a turn for user-facing narrative flow.
 *
 * System-prompt items (acpSource "session/new") are keyed per source-session
 * event identity (channel + seq + timestamp) so each session retains its own
 * card. They render as standalone single blocks at the head of their session
 * run — before the run's first turn — and never enter the prompt segment or
 * the CheckCheck (Prompt context) dialog.
 *
 * When items span multiple sessions (archived history + live session), a
 * `session-boundary` block is injected between consecutive session runs.
 * The newest-visible run is labeled distinctly when it equals
 * `latestLiveSessionId`, signalling "current session" vs archived history.
 * No boundary is emitted when only one session run is present.
 *
 * @param latestLiveSessionId - The session ID that is currently live on the
 *   relay (from `observerRelayStore`). Used to distinguish "current session"
 *   from "most recent observed session". Pass `null` when the relay is idle.
 */
export function buildTranscriptDisplayBlocks(
  items: TranscriptItem[],
  latestLiveSessionId: string | null = null,
): TranscriptDisplayBlock[] {
  const sessionRuns = splitIntoSessionRuns(items);

  // Fast path: single session (or zero) — no boundary blocks needed.
  if (sessionRuns.length <= 1) {
    return buildBlocksForRun(
      sessionRuns[0]?.items ?? [],
      /* isNewestRun */ true,
      sessionRuns[0]?.sessionId ?? null,
      latestLiveSessionId,
      /* emitBoundary */ false,
    );
  }

  // Multi-session: build blocks per run and interleave session-boundary blocks.
  const allBlocks: TranscriptDisplayBlock[] = [];
  for (let i = 0; i < sessionRuns.length; i++) {
    const run = sessionRuns[i];
    const isNewestRun = i === sessionRuns.length - 1;

    // Inject boundary before this run's blocks (not before the oldest run).
    if (i > 0) {
      const firstItem = run.items.find((it) => it.timestamp);
      const sessionStartTimestamp =
        firstItem?.timestamp ?? new Date(0).toISOString();
      const labelState: "current" | "most-recent" | "earlier" =
        isNewestRun &&
        latestLiveSessionId !== null &&
        run.sessionId === latestLiveSessionId
          ? "current"
          : isNewestRun
            ? "most-recent"
            : "earlier";
      // firstItemId: the id of the first TranscriptItem in this run. Stable
      // across prepend — inserting older runs before this run does not change
      // its first item. Falls back to sessionId when the run has no items
      // (edge case for empty runs from pre-session null-id buffers).
      const firstItemId = run.items[0]?.id ?? run.sessionId;
      allBlocks.push({
        kind: "session-boundary",
        sessionId: run.sessionId,
        sessionStartTimestamp,
        labelState,
        runIndex: i,
        firstItemId,
      });
    }

    const runBlocks = buildBlocksForRun(
      run.items,
      isNewestRun,
      run.sessionId,
      latestLiveSessionId,
      /* emitBoundary */ false,
    );
    allBlocks.push(...runBlocks);
  }

  return allBlocks;
}

/**
 * Build display blocks for a single session run's items.
 * Internal helper — extracted so `buildTranscriptDisplayBlocks` can call it
 * once per run without code duplication.
 */
function buildBlocksForRun(
  items: TranscriptItem[],
  _isNewestRun: boolean,
  _sessionId: string | null,
  _latestLiveSessionId: string | null,
  _emitBoundary: boolean,
): TranscriptDisplayBlock[] {
  const blocks: TranscriptDisplayBlock[] = [];
  const turnBuckets = new Map<string, TurnBucket>();
  // Callers pass a channel-scoped item stream; revisit this bare turnId bucket
  // if grouping ever receives multi-channel transcript items.
  const displayOrder: Array<
    { kind: "single"; item: TranscriptItem } | { kind: "turn"; turnId: string }
  > = [];

  // Strategy: accumulate null-session items into `openBatch` while no turn has
  // been seen. The batch opens on the first system-prompt item. When the first
  // turn-bound item is encountered (whether it creates a new bucket or reuses
  // one already started earlier — e.g. turn_started → session/new →
  // session_resolved, all on the same turn ID), the open batch is bound to that
  // turn ID in `pendingForTurn` and sealed (openBatch = null) so subsequent
  // null-session items fall through to inline emission and are NOT hoisted to
  // the session head. Append to any existing batch for that turn rather than
  // overwrite so a repeated seal on the same turn cannot lose an earlier batch.
  // End-of-stream fallback: if no turn ever arrives, emit the open batch as
  // standalone singles so the cards remain visible.
  let openBatch: TranscriptItem[] | null = null;
  // Maps a turn ID to the items that must flush before that turn in the render pass.
  const pendingForTurn = new Map<string, TranscriptItem[]>();

  for (const item of items) {
    const turnId = item.turnId;
    if (!turnId) {
      if (openBatch !== null) {
        // Pre-turn buffering is active — accumulate in wire order.
        openBatch.push(item);
      } else if (isSystemPrompt(item)) {
        // First system-prompt seen and no turn yet — open the batch.
        openBatch = [item];
      } else {
        // No pending batch, or batch already sealed; emit inline.
        displayOrder.push({ kind: "single", item });
      }
      continue;
    }

    // Seal the open batch on the FIRST turn-bound item encountered while
    // buffering is active — new bucket or existing (e.g. session_resolved
    // reusing the turn_started bucket).
    if (openBatch !== null) {
      const existing = pendingForTurn.get(turnId);
      if (existing) {
        existing.push(...openBatch);
      } else {
        pendingForTurn.set(turnId, openBatch);
      }
      openBatch = null;
    }

    let bucket = turnBuckets.get(turnId);
    if (!bucket) {
      bucket = { turnId, items: [] };
      turnBuckets.set(turnId, bucket);
      displayOrder.push({ kind: "turn", turnId });
    }
    bucket.items.push(item);
  }

  for (const entry of displayOrder) {
    if (entry.kind === "single") {
      blocks.push({ kind: "single", item: entry.item });
      continue;
    }

    const bucket = turnBuckets.get(entry.turnId);
    if (!bucket || bucket.items.length === 0) {
      continue;
    }

    // Flush any items bound to this turn (system-prompt card(s) plus any
    // interleaved null-session frames) as standalone blocks before the turn.
    const beforeTurn = pendingForTurn.get(entry.turnId);
    if (beforeTurn) {
      for (const pending of beforeTurn) {
        blocks.push({ kind: "single", item: pending });
      }
    }

    const segments = classifyTurnItems(bucket.items);
    if (segments.length > 0) {
      blocks.push({
        kind: "turn",
        turnId: entry.turnId,
        segments,
      });
    }
  }

  // End-of-stream fallback: session/new arrived but no turn followed yet
  // (incomplete stream or mid-restart). Emit as standalone singles so the
  // cards remain visible and are not silently dropped.
  if (openBatch !== null) {
    for (const pending of openBatch) {
      blocks.push({ kind: "single", item: pending });
    }
  }

  return blocks;
}

/** Flatten display blocks back to items for testing display order. */
export function flattenDisplayBlocks(
  blocks: TranscriptDisplayBlock[],
): TranscriptItem[] {
  const result: TranscriptItem[] = [];

  for (const block of blocks) {
    if (block.kind === "single") {
      result.push(block.item);
      continue;
    }

    // session-boundary blocks carry no items — skip.
    if (block.kind === "session-boundary") {
      continue;
    }

    for (const segment of block.segments) {
      if (segment.kind === "item") {
        result.push(segment.item);
      } else if (segment.kind === "prompt") {
        result.push(segment.user);
        result.push(...segment.setup);
        if (segment.context) {
          result.push(segment.context);
        }
      } else if (segment.kind === "summary") {
        result.push(...segment.summary.items);
      } else {
        result.push(...segment.items);
      }
    }
  }

  return result;
}

/**
 * Stable display key for a transcript block — used as the React list key and
 * as the `data-message-id` attribute that `useAnchoredScroll` anchors on.
 *
 * Must be kept in sync with the `data-message-id` rendered by
 * `AgentSessionTranscriptList` so outer scroll-anchor ids and inner DOM ids
 * always agree 1:1.
 */
export function getDisplayBlockKey(block: TranscriptDisplayBlock): string {
  if (block.kind === "single") {
    return block.item.id;
  }
  if (block.kind === "session-boundary") {
    // Use firstItemId (stable across prepend) rather than runIndex (shifts when
    // older sessions are prepended, causing unnecessary boundary remounts).
    return `session-boundary:${block.sessionId}:${block.firstItemId}`;
  }
  return `turn:${block.turnId}`;
}

/**
 * Derive the ordered display-block key sequence from raw observer events.
 *
 * Pure function: `buildTranscriptState(events).items` →
 * `buildTranscriptDisplayBlocks` → `getDisplayBlockKey`. This is the exact
 * chain `AgentSessionThreadPanel` uses to produce the id list fed to
 * `useAnchoredScroll` — extracted so both production and tests share one
 * code path.
 *
 * `latestLiveSessionId` is intentionally omitted: it only affects boundary
 * `labelState`, never keys.
 */
export function deriveTranscriptBlockIds(
  events: readonly ObserverEvent[],
): string[] {
  const items = buildTranscriptState(events).items;
  const blocks = buildTranscriptDisplayBlocks(items);
  return blocks.map(getDisplayBlockKey);
}

/** Human-readable labels for a collapsed turn setup row. */
export function formatTurnSetupLabel(
  items: Extract<TranscriptItem, { type: "lifecycle" }>[],
): string {
  const labels = items.map((item) => item.title);
  return labels.join(" · ");
}

/** Earliest timestamp among setup lifecycle items. */
export function turnSetupTimestamp(
  items: Extract<TranscriptItem, { type: "lifecycle" }>[],
): string | null {
  if (items.length === 0) return null;
  return items.reduce(
    (earliest, item) =>
      Date.parse(item.timestamp) < Date.parse(earliest)
        ? item.timestamp
        : earliest,
    items[0].timestamp,
  );
}

/** Optional detail text from setup lifecycle items (e.g. trigger count). */
export function turnSetupDetail(
  items: Extract<TranscriptItem, { type: "lifecycle" }>[],
): string | null {
  const details = items
    .map((item) => item.text.trim())
    .filter((text) => text.length > 0);
  if (details.length === 0) return null;
  return details.join(" ");
}
