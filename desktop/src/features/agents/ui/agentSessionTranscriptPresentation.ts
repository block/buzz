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

/** Max length for composer / View-all activity headlines. */
export const ACTIVITY_HEADLINE_MAX = 72;

/**
 * Collapse whitespace on the first line and ellipsize to `max` chars.
 * Prevents shell/tool payload dumps from exploding overview UIs.
 */
export function clampHeadline(
  text: string,
  max = ACTIVITY_HEADLINE_MAX,
): string {
  const firstLine = text.split("\n")[0]?.replace(/\s+/g, " ").trim() ?? "";
  if (firstLine.length === 0) {
    return "";
  }
  if (firstLine.length <= max) {
    return firstLine;
  }
  return `${firstLine.slice(0, Math.max(0, max - 3))}…`;
}

/** Human-readable headline for a single transcript item. */
export function getActivityHeadline(item: TranscriptItem): string | null {
  if (item.type === "tool") {
    const summary = buildCompactToolSummary(item);
    const joined = [summary.label, summary.preview].filter(Boolean).join(" · ");
    return joined ? clampHeadline(joined) : null;
  }

  if (item.type === "message") {
    if (item.role === "assistant") {
      const trimmed = item.text.trim();
      if (trimmed.length > 0) {
        const clamped = clampHeadline(trimmed);
        if (clamped.length > 0) {
          return clamped;
        }
      }
      return "Responding";
    }
    return clampHeadline(item.title || "User prompt");
  }

  if (item.type === "thought") {
    return clampHeadline(item.title === "Plan" ? "Planning" : item.title);
  }

  if (item.type === "metadata") {
    return clampHeadline(item.title);
  }

  return clampHeadline(item.title);
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
