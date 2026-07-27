/**
 * Live status line for harness mode — the "what is it doing right now" strip.
 *
 * Everything here is derived from data the observer stream already carries, so
 * nothing is estimated or faked:
 * - tool items expose `toolName`, `args`, `status`, `startedAt`, `completedAt`
 * - `usage_update` frames become a lifecycle row whose text holds the token
 *   counters and (when the provider reports it) real spend
 * - `turn_started` gives the turn's clock origin
 */

export type HarnessStatusItem = {
  id: string;
  type: string;
  role?: string;
  /** Source event id for a user row, when the transcript resolved one. */
  messageId?: string | null;
  renderClass?: string;
  title?: string;
  text?: string;
  toolName?: string;
  status?: string;
  args?: Record<string, unknown>;
  timestamp: string;
  startedAt?: string | null;
  completedAt?: string | null;
};

export type HarnessStatus = {
  /** Shell commands currently executing, in start order. */
  runningCommands: string[];
  /** Total tool calls in this turn, and how many have finished. */
  toolsTotal: number;
  toolsDone: number;
  /** Tokens used / context size, when a usage frame has arrived. */
  tokensUsed: number | null;
  tokensSize: number | null;
  /** Provider-reported spend for the turn, already formatted. */
  cost: string | null;
  /** Most recent thought or plan text — the short "what it's up to" line. */
  summary: string | null;
};

/**
 * Bee-flavoured progress words. Cycled by index so the caller controls cadence
 * (and so tests stay deterministic — no clock or randomness in here).
 */
export const HARNESS_BUZZWORDS = [
  "Buzzing",
  "Zipping",
  "Pollinating",
  "Foraging",
  "Nectaring",
  "Swarming",
  "Waggling",
  "Combing",
  "Fermenting",
  "Humming",
  "Beelining",
  "Hiving",
] as const;

export function buzzwordAt(tick: number): string {
  const index =
    ((tick % HARNESS_BUZZWORDS.length) + HARNESS_BUZZWORDS.length) %
    HARNESS_BUZZWORDS.length;
  return HARNESS_BUZZWORDS[index];
}

/** `Tokens: 32048/1000000 ($0.1754 USD)` → counts + formatted cost. */
export function parseUsageText(text: string | undefined | null): {
  used: number | null;
  size: number | null;
  cost: string | null;
} {
  if (!text) {
    return { used: null, size: null, cost: null };
  }
  const counts = text.match(/(\d+)\s*\/\s*(\d+)/);
  const cost = text.match(/\(\$([0-9.]+)\s*([A-Za-z]{3})\)/);
  return {
    used: counts ? Number(counts[1]) : null,
    size: counts ? Number(counts[2]) : null,
    cost: cost ? `$${cost[1]}` : null,
  };
}

/** Best-effort shell command text from a tool call's arguments. */
export function shellCommandOf(item: HarnessStatusItem): string | null {
  const args = item.args ?? {};
  for (const key of ["command", "cmd", "script"]) {
    const value = args[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

export function deriveHarnessStatus(
  items: readonly HarnessStatusItem[],
): HarnessStatus {
  const tools = items.filter((item) => item.type === "tool");
  const running = tools.filter((item) => item.status === "executing");

  // Latest usage frame wins: `usage:<channel>:<turn>` is replaced in place as
  // the turn progresses, so the last one holds current totals.
  const usage = [...items]
    .reverse()
    .find((item) => item.id.startsWith("usage:"));
  const parsed = parseUsageText(usage?.text);

  // Prefer the newest thought, then plan — the closest thing to a one-line
  // "what it's doing" that the agent itself produced.
  const summarySource = [...items]
    .reverse()
    .find((item) => item.type === "thought" || item.type === "plan");

  return {
    runningCommands: running
      .map(shellCommandOf)
      .filter((command): command is string => command !== null),
    toolsTotal: tools.length,
    toolsDone: tools.filter((item) => item.status !== "executing").length,
    tokensUsed: parsed.used,
    tokensSize: parsed.size,
    cost: parsed.cost,
    summary: firstLine(summarySource?.text) ?? null,
  };
}

function firstLine(text: string | undefined | null): string | null {
  if (!text) {
    return null;
  }
  const line = text
    .split("\n")
    .find((candidate) => candidate.trim().length > 0);
  return line ? line.trim() : null;
}

/** `77000` → `1m 17s`; `3200` → `3s`. Mirrors Claude Code's compact form. */
export function formatElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes > 0 ? `${minutes}m ${seconds}s` : `${seconds}s`;
}

/** `3200` → `3.2k`; `980` → `980`. */
export function formatTokens(count: number): string {
  if (count < 1000) {
    return String(count);
  }
  return `${(count / 1000).toFixed(1)}k`;
}
