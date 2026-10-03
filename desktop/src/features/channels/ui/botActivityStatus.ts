import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

/**
 * Stable, Slack-style status line for the composer activity bar.
 *
 * The bar used to rotate through the last few distinct transcript headlines,
 * which read as "3-4 things flickering in a loop" during a long agent turn.
 * Instead we derive ONE line that updates in place:
 *
 *   Assistant · Bash: npm test · 2m 10s · 12 tools · ctx 118K/1M
 *
 * Everything here is a pure projection of the transcript; the elapsed-time
 * segment is rendered by the component (it needs a ticking clock).
 */

export type StableActivityStatus = {
  /** What the agent is doing right now ("Bash: npm test", "Thinking", …). */
  activity: string;
  /** Tool calls in the current turn (suppressed rows excluded). */
  toolCount: number;
  /** Compact context reading ("118K/1M"), when a usage frame has arrived. */
  context: string | null;
};

/** Preview clamp keeps the trailing counters visible in a truncating row. */
const MAX_PREVIEW_CHARS = 40;

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

function isRunning(item: ToolItem): boolean {
  return item.status === "executing" || item.status === "pending";
}

function isCountableTool(item: TranscriptItem): item is ToolItem {
  return item.type === "tool" && item.renderClass !== "suppressed";
}

/** Absolute paths shorten to their basename; anything else passes through. */
function shortPreview(preview: string): string {
  let out = preview;
  if (out.startsWith("/") || out.startsWith("~")) {
    const base = out.split("/").filter(Boolean).at(-1);
    if (base) {
      out = base;
    }
  }
  if (out.length > MAX_PREVIEW_CHARS) {
    out = `${out.slice(0, MAX_PREVIEW_CHARS - 1)}…`;
  }
  return out;
}

/**
 * "Bash: Run the test suite" — the emitted tool title plus a clamped preview.
 * A human-authored `description` argument (Claude Code sends one with every
 * shell call) beats the raw command line: the bar is a status line, not a
 * terminal, and `sed -n '420,432p' …` reads as noise there. The session panel
 * still shows the full command.
 */
function toolActivity(item: ToolItem): string {
  const description = item.args?.description;
  if (typeof description === "string" && description.trim().length > 0) {
    return `${item.title}: ${shortPreview(description.trim())}`;
  }
  const preview = item.descriptor?.preview;
  if (typeof preview === "string" && preview.trim().length > 0) {
    return `${item.title}: ${shortPreview(preview.trim())}`;
  }
  return item.title;
}

function phaseActivity(item: TranscriptItem): string | null {
  if (isCountableTool(item)) {
    return toolActivity(item);
  }
  if (item.type === "thought") {
    return "Thinking";
  }
  if (item.type === "plan") {
    return "Planning";
  }
  if (item.type === "message" && item.role === "assistant") {
    return "Responding";
  }
  return null;
}

/** 87_500 → "88K", 1_000_000 → "1M", 1_500_000 → "1.5M", 950 → "950". */
export function formatTokens(count: number): string {
  if (count >= 1_000_000) {
    const millions = count / 1_000_000;
    const rounded = Math.round(millions * 10) / 10;
    return Number.isInteger(rounded) ? `${rounded}M` : `${rounded.toFixed(1)}M`;
  }
  if (count >= 1_000) {
    return `${Math.round(count / 1_000)}K`;
  }
  return `${count}`;
}

/** 42_000ms → "42s", 130_000 → "2m 10s", 3_840_000 → "1h 4m". */
export function formatElapsed(elapsedMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000));
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return `${totalMinutes}m ${totalSeconds % 60}s`;
  }
  return `${Math.floor(totalMinutes / 60)}h ${totalMinutes % 60}m`;
}

const USAGE_TEXT = /Tokens:\s*(\d+)\/(\d+)/;

/**
 * Project the (channel-scoped) transcript into one stable status reading.
 * The current turn is whichever turnId the newest attributed item carries;
 * counters are scoped to it so a fresh turn starts back at zero.
 */
export function buildStableActivityStatus(
  transcript: TranscriptItem[],
  channelId: string | null,
  threadRootId: string | null = null,
): StableActivityStatus {
  const channelScoped = channelId
    ? transcript.filter((item) => item.channelId === channelId)
    : transcript;
  // Thread bars lock onto their own turn: the emitting harness stamps items
  // with the thread root shortened as sessionId, so prefix match selects them.
  const scoped = threadRootId
    ? channelScoped.filter(
        (item) =>
          typeof item.sessionId === "string" &&
          item.sessionId.length > 0 &&
          threadRootId.startsWith(item.sessionId),
      )
    : channelScoped;

  let currentTurnId: string | null = null;
  for (let i = scoped.length - 1; i >= 0; i--) {
    const turnId = scoped[i]?.turnId;
    if (typeof turnId === "string" && turnId.length > 0) {
      currentTurnId = turnId;
      break;
    }
  }

  const turnItems = currentTurnId
    ? scoped.filter((item) => item.turnId === currentTurnId)
    : [];

  let toolCount = 0;
  let runningTool: ToolItem | null = null;
  for (const item of turnItems) {
    if (!isCountableTool(item)) {
      continue;
    }
    toolCount += 1;
    if (isRunning(item)) {
      runningTool = item; // newest running tool wins
    }
  }

  let activity = runningTool ? toolActivity(runningTool) : null;
  if (!activity) {
    for (let i = turnItems.length - 1; i >= 0 && !activity; i--) {
      const item = turnItems[i];
      if (item) {
        activity = phaseActivity(item);
      }
    }
  }

  // Context survives across turns (it is a session-level reading), so the
  // newest usage item in the whole scope wins, not just the current turn.
  let context: string | null = null;
  for (let i = scoped.length - 1; i >= 0; i--) {
    const item = scoped[i];
    if (item?.type !== "lifecycle" || !item.id.startsWith("usage:")) {
      continue;
    }
    const match = USAGE_TEXT.exec(item.text);
    if (match?.[1] && match[2]) {
      context = `${formatTokens(Number(match[1]))}/${formatTokens(Number(match[2]))}`;
    }
    break;
  }

  return {
    activity: activity ?? "Working",
    toolCount,
    context,
  };
}

/** Join the per-turn segments the bar renders after the agent's name. */
export function formatStatusSegments(
  status: StableActivityStatus,
  elapsed: string | null,
): string {
  const segments = [status.activity];
  if (elapsed) {
    segments.push(elapsed);
  }
  if (status.toolCount > 0) {
    segments.push(
      `${status.toolCount} ${status.toolCount === 1 ? "tool" : "tools"}`,
    );
  }
  if (status.context) {
    segments.push(`ctx ${status.context}`);
  }
  return segments.join(" · ");
}
