import type { TranscriptItem } from "./agentSessionTypes";
import { classifyToolItem } from "./agentSessionToolClassifier";

export type TranscriptTurnSegment =
  | { kind: "item"; item: TranscriptItem }
  | { kind: "summary"; summary: TranscriptToolRunSummary }
  | { kind: "setup"; items: Extract<TranscriptItem, { type: "lifecycle" }>[] }
  | {
      kind: "prompt";
      user: Extract<TranscriptItem, { type: "message" }>;
      systemPrompt: Extract<TranscriptItem, { type: "metadata" }> | null;
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
       */
      runIndex: number;
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

function classifyTurnItems(
  items: TranscriptItem[],
  externalSystemPrompt: Extract<
    TranscriptItem,
    { type: "metadata" }
  > | null = null,
): TranscriptTurnSegment[] {
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
        systemPrompt: null,
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
      systemPrompt: externalSystemPrompt,
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
 * session run as the turn they belong to and the `pendingSystemPrompt` slot in
 * `buildBlocksForRun` can consume them correctly.
 *
 * Mid-stream null-session items (after at least one session has resolved) are
 * attributed to the most recently seen session run — same as before, this
 * handles gap frames that arrive after resolution.
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

  for (const item of items) {
    if (item.sessionId === null || item.sessionId === undefined) {
      if (currentRun === null) {
        // No session resolved yet — defer into the pre-session buffer.
        preSessionBuffer.push(item);
      } else {
        // Session already resolved — attribute to current run.
        currentRun.items.push(item);
      }
      continue;
    }

    // item.sessionId is non-null from here.
    if (!currentRun || item.sessionId !== currentRun.sessionId) {
      const newRun: { sessionId: string; items: TranscriptItem[] } = {
        sessionId: item.sessionId,
        items: [],
      };
      if (currentRun === null && preSessionBuffer.length > 0) {
        // First resolved session: prepend buffered pre-resolution items.
        newRun.items.push(...preSessionBuffer);
        preSessionBuffer.length = 0;
      }
      currentRun = newRun;
      runs.push(currentRun);
    }
    currentRun.items.push(item);
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
 * System-prompt items (acpSource "session/new") are per-channel singles with
 * turnId=null. They are injected into the prompt segment of the first turn
 * that follows them in stream order — placing System prompt between the user
 * message bubble and the Prompt context sections in the rendered output.
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
      allBlocks.push({
        kind: "session-boundary",
        sessionId: run.sessionId,
        sessionStartTimestamp,
        labelState,
        runIndex: i,
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

  // System-prompt items (turnId=null, acpSource "session/new") accumulate here
  // until consumed by the first turn that follows them in stream order.
  let pendingSystemPrompt: Extract<
    TranscriptItem,
    { type: "metadata" }
  > | null = null;

  for (const item of items) {
    const turnId = item.turnId;
    if (!turnId) {
      if (isSystemPrompt(item)) {
        // Hold system-prompt for injection into the next turn's prompt segment.
        pendingSystemPrompt = item;
      } else {
        displayOrder.push({ kind: "single", item });
      }
      continue;
    }

    let bucket = turnBuckets.get(turnId);
    if (!bucket) {
      bucket = { turnId, items: [] };
      turnBuckets.set(turnId, bucket);
      displayOrder.push({ kind: "turn", turnId });
    }
    bucket.items.push(item);
  }

  // Track per-turn injected system-prompt so multi-turn streams don't re-inject.
  const consumedSystemPrompts = new Set<string>();

  for (const entry of displayOrder) {
    if (entry.kind === "single") {
      blocks.push({ kind: "single", item: entry.item });
      continue;
    }

    const bucket = turnBuckets.get(entry.turnId);
    if (!bucket || bucket.items.length === 0) {
      continue;
    }

    // Inject system-prompt into the first turn that has a user-prompt item.
    // On subsequent turns, system-prompt stays null (session/new doesn't re-fire).
    let systemPromptForTurn: Extract<
      TranscriptItem,
      { type: "metadata" }
    > | null = null;
    if (
      pendingSystemPrompt &&
      !consumedSystemPrompts.has(pendingSystemPrompt.id) &&
      bucket.items.some(isUserPrompt)
    ) {
      systemPromptForTurn = pendingSystemPrompt;
      consumedSystemPrompts.add(pendingSystemPrompt.id);
    }

    const segments = classifyTurnItems(bucket.items, systemPromptForTurn);
    if (segments.length > 0) {
      blocks.push({
        kind: "turn",
        turnId: entry.turnId,
        segments,
      });
    }
  }

  // If system-prompt was never consumed (no session/prompt followed — e.g.
  // session/new arrived without a subsequent turn, or the stream is still
  // incomplete), emit it as a standalone single so it remains visible.
  if (
    pendingSystemPrompt &&
    !consumedSystemPrompts.has(pendingSystemPrompt.id)
  ) {
    blocks.push({ kind: "single", item: pendingSystemPrompt });
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
        if (segment.systemPrompt) {
          result.push(segment.systemPrompt);
        }
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
