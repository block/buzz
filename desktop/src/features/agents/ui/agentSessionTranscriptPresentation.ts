import { formatToolTitle } from "./agentSessionToolCatalog";
import type { TranscriptItem } from "./agentSessionTypes";

export type TranscriptActivityCounts = {
  tools: number;
  toolErrors: number;
  thoughts: number;
  messages: number;
  lifecycle: number;
  metadata: number;
};

export type TranscriptActivityState =
  | "idle"
  | "responding"
  | "thinking"
  | "tool_running"
  | "error";

export type TranscriptPresentation = {
  headline: string;
  state: TranscriptActivityState;
  counts: TranscriptActivityCounts;
  latestMeaningfulItem: TranscriptItem | null;
  latestMeaningfulItemId: string | null;
  activeItemIds: ReadonlySet<string>;
  lastUpdatedAt: string | null;
  hasError: boolean;
};

const LIFECYCLE_NOISE = new Set([
  "turn started",
  "session ready",
  "wire parse error",
]);

/** Human-readable headline for a single transcript item. */
export function getActivityHeadline(item: TranscriptItem): string | null {
  if (item.type === "tool") {
    return formatToolTitle(item.buzzToolName ?? item.toolName, item.title);
  }

  if (item.type === "message") {
    if (item.role === "assistant") {
      const trimmed = item.text.trim();
      if (trimmed.length > 0) {
        const firstLine = trimmed.split("\n")[0]?.trim() ?? "";
        if (firstLine.length > 0) {
          return firstLine.length > 72
            ? `${firstLine.slice(0, 69)}…`
            : firstLine;
        }
      }
      return "Responding";
    }
    return item.title || "User prompt";
  }

  if (item.type === "thought") {
    return item.title === "Plan" ? "Planning" : item.title;
  }

  if (item.type === "metadata") {
    return item.title;
  }

  return item.title;
}

function isLifecycleNoise(
  item: Extract<TranscriptItem, { type: "lifecycle" }>,
) {
  return LIFECYCLE_NOISE.has(item.title.toLowerCase());
}

/** Whether an item should contribute to the "Now" summary and headline scan. */
export function isMeaningfulItem(item: TranscriptItem): boolean {
  if (item.type === "lifecycle") {
    return !isLifecycleNoise(item);
  }
  if (item.type === "metadata") {
    return false;
  }
  return true;
}

function isToolRunning(item: Extract<TranscriptItem, { type: "tool" }>) {
  return item.status === "executing" || item.status === "pending";
}

function isLifecycleError(
  item: Extract<TranscriptItem, { type: "lifecycle" }>,
) {
  return item.title.toLowerCase().includes("error");
}

function countItems(items: TranscriptItem[]): TranscriptActivityCounts {
  const counts: TranscriptActivityCounts = {
    tools: 0,
    toolErrors: 0,
    thoughts: 0,
    messages: 0,
    lifecycle: 0,
    metadata: 0,
  };

  for (const item of items) {
    switch (item.type) {
      case "tool":
        counts.tools += 1;
        if (item.isError || item.status === "failed") {
          counts.toolErrors += 1;
        }
        break;
      case "thought":
        counts.thoughts += 1;
        break;
      case "message":
        counts.messages += 1;
        break;
      case "lifecycle":
        counts.lifecycle += 1;
        break;
      case "metadata":
        counts.metadata += 1;
        break;
    }
  }

  return counts;
}

function findLatestMeaningfulItem(
  items: TranscriptItem[],
): TranscriptItem | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (isMeaningfulItem(item)) {
      return item;
    }
  }
  return null;
}

function resolveActivityState(
  latest: TranscriptItem | null,
  hasError: boolean,
  isWorking: boolean,
): TranscriptActivityState {
  if (!isWorking) {
    return hasError ? "error" : "idle";
  }

  if (hasError && latest?.type === "lifecycle" && isLifecycleError(latest)) {
    return "error";
  }

  if (latest?.type === "tool" && isToolRunning(latest)) {
    return "tool_running";
  }

  if (latest?.type === "thought") {
    return "thinking";
  }

  if (latest?.type === "message" && latest.role === "assistant") {
    return "responding";
  }

  if (latest?.type === "tool") {
    return "tool_running";
  }

  return "idle";
}

function resolveHeadline(
  latest: TranscriptItem | null,
  state: TranscriptActivityState,
  isWorking: boolean,
): string {
  if (latest) {
    const headline = getActivityHeadline(latest);
    if (headline) {
      return headline;
    }
  }

  if (isWorking) {
    switch (state) {
      case "tool_running":
        return "Running a tool";
      case "thinking":
        return "Thinking";
      case "responding":
        return "Responding";
      case "error":
        return "Encountered an error";
      default:
        return "Working";
    }
  }

  if (state === "error") {
    return "Last turn ended with an error";
  }

  return "Waiting for activity";
}

function collectActiveItemIds(
  items: TranscriptItem[],
  isWorking: boolean,
): ReadonlySet<string> {
  if (!isWorking || items.length === 0) {
    return new Set();
  }

  const active = new Set<string>();

  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];

    if (item.type === "tool" && isToolRunning(item)) {
      active.add(item.id);
      break;
    }

    if (item.type === "thought") {
      active.add(item.id);
      break;
    }

    if (item.type === "message" && item.role === "assistant") {
      active.add(item.id);
      break;
    }
  }

  return active;
}

function detectError(items: TranscriptItem[]): boolean {
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (!isMeaningfulItem(item)) {
      continue;
    }
    if (item.type === "lifecycle" && isLifecycleError(item)) {
      return true;
    }
    if (item.type === "tool" && (item.isError || item.status === "failed")) {
      return true;
    }
    break;
  }
  return false;
}

/** Derive presentation metadata for a transcript list. */
export function buildTranscriptPresentation(
  items: TranscriptItem[],
  isWorking = false,
): TranscriptPresentation {
  const latestMeaningfulItem = findLatestMeaningfulItem(items);
  const hasError = detectError(items);
  const state = resolveActivityState(latestMeaningfulItem, hasError, isWorking);

  return {
    headline: resolveHeadline(latestMeaningfulItem, state, isWorking),
    state,
    counts: countItems(items),
    latestMeaningfulItem,
    latestMeaningfulItemId: latestMeaningfulItem?.id ?? null,
    activeItemIds: collectActiveItemIds(items, isWorking),
    lastUpdatedAt:
      items.length > 0 ? (items[items.length - 1]?.timestamp ?? null) : null,
    hasError,
  };
}
