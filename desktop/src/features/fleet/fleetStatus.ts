/**
 * Pure derivations for the fleet board: per-agent live status, last-seen
 * timestamps, activity headlines, and compact number formatting. Free of
 * React/Tauri imports so the rules are unit-testable.
 */

import type { ManagedAgent } from "@/shared/api/types";
import type {
  ObserverEvent,
  TranscriptItem,
} from "@/features/agents/ui/agentSessionTypes";

export type FleetLiveStatus = "working" | "idle" | "error" | "offline";

/**
 * Collapse the working signal, managed-agent process status, and last error
 * into one glanceable state:
 *
 * - `working` — observer-derived active turn (or typing fallback) right now.
 * - `idle`    — process is up (`running`/`deployed`) but no active turn.
 * - `error`   — process is down and the harness recorded an error on the way
 *               out; surfaces crashed/auth-failed agents that need attention.
 * - `offline` — stopped/not deployed with no recorded error.
 */
export function deriveFleetLiveStatus(input: {
  working: boolean;
  status: ManagedAgent["status"];
  lastError: string | null;
}): FleetLiveStatus {
  if (input.working) return "working";
  if (input.status === "running" || input.status === "deployed") return "idle";
  return input.lastError ? "error" : "offline";
}

/**
 * Newest activity timestamp for an agent, in desktop-clock ms:
 * the latest observer frame (events arrive sorted ascending, so the last one
 * wins) or the managed-agent lifecycle timestamps, whichever is newest.
 * Returns null when nothing parseable exists.
 */
export function deriveLastSeenMs(input: {
  events: readonly ObserverEvent[];
  lastStartedAt?: string | null;
  lastStoppedAt?: string | null;
}): number | null {
  let latest: number | null = null;
  const candidates: Array<string | null | undefined> = [
    input.events.at(-1)?.timestamp,
    input.lastStartedAt,
    input.lastStoppedAt,
  ];
  for (const candidate of candidates) {
    if (!candidate) continue;
    const parsed = Date.parse(candidate);
    if (Number.isFinite(parsed) && (latest === null || parsed > latest)) {
      latest = parsed;
    }
  }
  return latest;
}

/** Transcript render classes that never make a meaningful headline. */
const HEADLINE_SKIP_CLASSES: ReadonlySet<string> = new Set([
  "suppressed",
  "raw-rail",
]);

/**
 * One-line "what is this agent doing / what did it last do" headline: the
 * newest transcript item that renders as real activity. Message items headline
 * with their text (the title is just the speaker role); everything else uses
 * its title ("Editing src/foo.ts", "Ran cargo test", ...).
 */
export function deriveActivityHeadline(
  transcript: readonly TranscriptItem[],
): string | null {
  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    const item = transcript[index];
    if (HEADLINE_SKIP_CLASSES.has(item.renderClass)) continue;
    const text =
      item.type === "message" ? item.text.trim() || item.title : item.title;
    const trimmed = text.trim();
    if (trimmed.length > 0) return trimmed;
  }
  return null;
}

/** Compact token count: 950 → "950", 12_340 → "12.3k", 4_200_000 → "4.2M". */
export function formatTokenCount(count: number): string {
  if (count < 1_000) return String(count);
  if (count < 1_000_000) {
    return `${trimTrailingZero((count / 1_000).toFixed(1))}k`;
  }
  return `${trimTrailingZero((count / 1_000_000).toFixed(1))}M`;
}

function trimTrailingZero(value: string): string {
  return value.endsWith(".0") ? value.slice(0, -2) : value;
}

/** Compact USD cost: 0 → "$0.00", 0.004 → "<$0.01", 1.5 → "$1.50". */
export function formatUsdCost(costUsd: number): string {
  if (costUsd > 0 && costUsd < 0.01) return "<$0.01";
  return `$${costUsd.toFixed(2)}`;
}

/** Relative last-seen label: "just now", "5m ago", "3h ago", "2d ago". */
export function formatLastSeen(lastSeenMs: number, nowMs: number): string {
  const elapsed = Math.max(0, nowMs - lastSeenMs);
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}
