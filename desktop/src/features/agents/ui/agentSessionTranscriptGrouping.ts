import type { TranscriptItem } from "./agentSessionTypes";
import { classifyToolItem } from "./agentSessionToolClassifier";

export type TranscriptTurnSegment =
  | { kind: "item"; item: TranscriptItem }
  | { kind: "summary"; summary: TranscriptSameKindSummary }
  | { kind: "setup"; items: Extract<TranscriptItem, { type: "lifecycle" }>[] }
  | {
      kind: "prompt";
      user: Extract<TranscriptItem, { type: "message" }>;
      context: Extract<TranscriptItem, { type: "metadata" }> | null;
      setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
    };

export type TranscriptDisplayBlock =
  | { kind: "single"; item: TranscriptItem }
  | { kind: "turn"; turnId: string; segments: TranscriptTurnSegment[] };

export type TranscriptSameKindSummary = {
  id: string;
  label: string;
  count: number;
  items: TranscriptItem[];
  renderClass: TranscriptItem["renderClass"] | null;
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

function isPromptContext(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "metadata" }> {
  return (
    item.type === "metadata" && item.acpSource === "session/prompt:context"
  );
}

function isSetupLifecycle(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "lifecycle" }> {
  return (
    item.type === "lifecycle" &&
    (item.acpSource === "turn_started" || item.acpSource === "session_resolved")
  );
}

function isErrorLifecycle(
  item: TranscriptItem,
): item is Extract<TranscriptItem, { type: "lifecycle" }> {
  return (
    item.type === "lifecycle" && item.title.toLowerCase().includes("error")
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
  const consumed = new Set<TranscriptItem>();

  if (userPrompt) consumed.add(userPrompt);
  for (const item of setupLifecycle) consumed.add(item);
  if (promptContext) consumed.add(promptContext);

  const activity = items.filter((item) => !consumed.has(item));

  if (!userPrompt) {
    return groupSameKindSegments(
      activity.map((item) => ({ kind: "item", item })),
    );
  }

  const segments: TranscriptTurnSegment[] = [
    {
      kind: "prompt",
      user: userPrompt,
      context: promptContext,
      setup: setupLifecycle,
    },
  ];

  for (const item of activity) {
    if (isErrorLifecycle(item)) {
      segments.push({ kind: "item", item });
      continue;
    }
    segments.push({ kind: "item", item });
  }

  return groupSameKindSegments(segments);
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

function sameKindKey(item: TranscriptItem): string | null {
  if (item.type !== "tool") return null;
  const renderClass = getRenderClass(item);
  if (renderClass === "message") {
    return null;
  }
  const descriptor = item.descriptor ?? classifyToolItem(item);
  return descriptor.groupKey ?? renderClass;
}

function sameKindLabel(item: TranscriptItem, count: number): string {
  if (item.type !== "tool") return `${count} items`;
  const descriptor = item.descriptor ?? classifyToolItem(item);
  const renderClass = getRenderClass(item);
  const label = descriptor.label;
  if (renderClass === "file-edit") {
    return `Edited ${count} file${count === 1 ? "" : "s"}`;
  }
  if (label === "Read file") return `Read ${count} files`;
  if (label === "Ran command") return `Ran ${count} commands`;
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
 * Build presentation-only display blocks from normalized transcript items.
 * Raw observer order is preserved in the source items; this only reorders
 * within a turn for user-facing narrative flow.
 */
export function buildTranscriptDisplayBlocks(
  items: TranscriptItem[],
): TranscriptDisplayBlock[] {
  const blocks: TranscriptDisplayBlock[] = [];
  const turnBuckets = new Map<string, TurnBucket>();
  const displayOrder: Array<
    { kind: "single"; item: TranscriptItem } | { kind: "turn"; turnId: string }
  > = [];

  for (const item of items) {
    const turnId = item.turnId;
    if (!turnId) {
      displayOrder.push({ kind: "single", item });
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

  for (const entry of displayOrder) {
    if (entry.kind === "single") {
      blocks.push({ kind: "single", item: entry.item });
      continue;
    }

    const bucket = turnBuckets.get(entry.turnId);
    if (!bucket || bucket.items.length === 0) {
      continue;
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
