import type { TranscriptItem } from "./agentSessionTypes";
import { basename } from "./agentSessionFileEditDiff";
import {
  buildCompactToolSummary,
  type CompactToolSummary,
} from "./agentSessionToolSummary";

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

/**
 * Render classes whose action object is a filesystem path — compact to the
 * basename for terse headlines. Shell commands are deliberately excluded:
 * they legitimately contain "/" (e.g. `cat src/foo.ts`) and must not be
 * reduced to their last path segment.
 */
const PATHLIKE_RENDER_CLASSES = new Set([
  "file-read",
  "file-edit",
  "skill-read",
]);

function terseActionObject(summary: CompactToolSummary): string | null {
  if (summary.fileEditSummary) {
    return summary.fileEditSummary.filename;
  }
  const object = summary.action?.object ?? null;
  if (
    object &&
    PATHLIKE_RENDER_CLASSES.has(summary.kind) &&
    /[\\/]/.test(object)
  ) {
    return basename(object);
  }
  return object;
}

/**
 * Human-readable headline for a single transcript item.
 *
 * Tool items use the terse action tier — verb + compact object (file paths
 * reduced to their basename), e.g. "Read foo.ts" rather than
 * "Read file · src/agents/ui/foo.ts" — so the composer pill's 200px cap
 * shows the informative part instead of ellipsizing a long preview.
 *
 * Action headlines are past-tense verb phrases throughout ("Read foo.ts",
 * "Thought", "Updated plan") — plan and thought items are normalized here
 * so no bare noun ("Plan") or present participle ("Thinking") leaks into
 * the pill next to the past-tense tool verbs.
 */
export function getActivityHeadline(item: TranscriptItem): string | null {
  if (item.type === "tool") {
    const summary = buildCompactToolSummary(item);
    if (summary.action) {
      return [summary.action.verb, terseActionObject(summary)]
        .filter(Boolean)
        .join(" ");
    }
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
    if (item.title === "Plan") {
      return "Planned";
    }
    return item.title === "Thinking" ? "Thought" : item.title;
  }

  if (item.type === "plan") {
    return item.isUpdate ? "Updated plan" : "Created plan";
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
