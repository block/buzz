import type { TranscriptItem } from "./agentSessionTypes";
import { buildCompactToolSummary } from "./agentSessionToolSummary";

/**
 * Whether a polished activity row should render the opt-in timestamp footer.
 * User message bubbles already render their own timestamp footer, so they are
 * excluded to avoid doubling up. Compact previews stay dense regardless of
 * the preference.
 */
export function shouldShowTranscriptRowTimestamp(
  item: TranscriptItem,
  options: { enabled: boolean; variant: string },
): boolean {
  if (!options.enabled || options.variant === "compactPreview") {
    return false;
  }
  if (item.type === "message" && item.role !== "assistant") {
    return false;
  }
  return true;
}

const LIFECYCLE_NOISE = new Set([
  "turn started",
  "session ready",
  "wire parse error",
]);

/** Human-readable headline for a single transcript item. */
export function getActivityHeadline(item: TranscriptItem): string | null {
  if (item.type === "tool") {
    const summary = buildCompactToolSummary(item);
    return [summary.label, summary.preview].filter(Boolean).join(" · ");
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

/** Whether an item should contribute to the headline scan (noise gate). */
export function isMeaningfulItem(item: TranscriptItem): boolean {
  if (item.type === "tool" && item.renderClass === "suppressed") {
    return false;
  }
  if (item.type === "lifecycle") {
    return !isLifecycleNoise(item);
  }
  if (item.type === "metadata") {
    // Raw JSON-RPC frames ("Raw ACP payload") are infrastructure noise; all
    // other metadata items (system prompt, prompt context) are semantically
    // meaningful and visible in the feed.
    return item.acpSource !== "raw_json_rpc";
  }
  return true;
}

/**
 * Whether an item is "spine" work — eligible to headline over setup/context.
 * Tools, messages, thoughts, plans, and meaningful lifecycle events qualify.
 * Metadata items (system prompt, prompt context) are reads that should recede
 * when real work is present; they are NOT spine items.
 *
 * Used by BotActivityBar for the two-tier headline scan:
 * 1. Collect spine headlines first.
 * 2. If none found, fall back to including metadata so the bar isn't empty at
 *    session start / idle.
 */
export function isSpineItem(item: TranscriptItem): boolean {
  if (!isMeaningfulItem(item)) return false;
  return item.type !== "metadata";
}

/**
 * Latest meaningful activity headline for a transcript, optionally scoped to
 * one channel. Prefers spine work; falls back to any meaningful item so early
 * session context can still surface. Returns null when nothing qualifies.
 */
export function getLatestActivityHeadline(
  items: readonly TranscriptItem[],
  channelId?: string | null,
): string | null {
  const scoped =
    channelId && channelId.length > 0
      ? items.filter((item) => item.channelId === channelId)
      : items;
  if (scoped.length === 0) {
    return null;
  }

  const passFilter: (item: TranscriptItem) => boolean = scoped.some(isSpineItem)
    ? isSpineItem
    : isMeaningfulItem;

  for (let i = scoped.length - 1; i >= 0; i--) {
    const item = scoped[i];
    if (!item || !passFilter(item)) {
      continue;
    }
    const headline = getActivityHeadline(item);
    if (headline) {
      return headline;
    }
  }
  return null;
}
