import { RefreshCw } from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { RestartDiffEntry, RestartChange } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

// ── Auto-restart copy ─────────────────────────────────────────────────────────

/**
 * Shared blurb strings for the auto-restart state. Exported so the Runtime-tab
 * banner can import the same constants — the two surfaces must never drift.
 */
export const AUTO_RESTART_ON_BLURB =
  "Configuration changed since this agent started. Buzz can restart it automatically after ~3 minutes idle, or stop and respawn it to apply now.";

export const AUTO_RESTART_OFF_BLURB =
  "Configuration changed since this agent started. Automatic restart is off for this agent — stop and respawn it to apply the changes.";

// ── Label helpers ─────────────────────────────────────────────────────────────

/**
 * Humanise a serde dotted-path field id into a display label.
 * `snake_case` → `"Snake case"`, `env.FOO` → `"FOO (env)"`.
 * Unknown paths render as-is — no per-field label map.
 */
function humaniseFieldId(field: string): string {
  if (field.startsWith("env.")) {
    const key = field.slice(4);
    return `${key} (env)`;
  }
  // snake_case → words, sentence case
  return field.replace(/_/g, " ").replace(/\b\w/, (c) => c.toUpperCase());
}

// ── Change description ────────────────────────────────────────────────────────

function formatJsonValue(v: unknown): string {
  if (v === null) return "null";
  if (v === undefined) return "—";
  if (typeof v === "string") return v;
  return JSON.stringify(v);
}

function ChangeDescription({ change }: { change: RestartChange }) {
  switch (change.kind) {
    case "value":
      return (
        <span>
          <span className="line-through opacity-60">
            {formatJsonValue(change.before)}
          </span>
          {" → "}
          <span>{formatJsonValue(change.after)}</span>
        </span>
      );
    case "text": {
      const before = change.before_chars ?? 0;
      const after = change.after_chars ?? 0;
      return (
        <span>
          {before} chars → {after} chars
        </span>
      );
    }
    case "masked":
      return (
        <span>
          <span className="line-through opacity-60">
            {change.before ?? "••••"}
          </span>
          {" → "}
          <span>{change.after ?? "••••"}</span>
        </span>
      );
    case "added":
      return <span>added</span>;
    case "removed":
      return <span>removed</span>;
    default:
      // Unknown kind — render nothing, remain type-safe at runtime
      return null;
  }
}

// ── Diff list ─────────────────────────────────────────────────────────────────

const TOOLTIP_CAP = 6;

/**
 * `tooltip` — renders inside the dark `bg-primary` tooltip; uses
 *   `text-primary-foreground` variants for contrast there.
 * `inline`  — renders inside the amber Runtime banner or other light
 *   surfaces; inherits foreground from the container instead.
 */
function DiffList({
  entries,
  cap,
  variant = "tooltip",
}: {
  entries: RestartDiffEntry[];
  cap?: number;
  variant?: "tooltip" | "inline";
}) {
  const visible = cap !== undefined ? entries.slice(0, cap) : entries;
  const overflow =
    cap !== undefined && entries.length > cap ? entries.length - cap : 0;

  const valueClass =
    variant === "tooltip" ? "text-primary-foreground/80" : "text-foreground";
  const overflowClass =
    variant === "tooltip"
      ? "text-primary-foreground/60"
      : "text-muted-foreground";

  return (
    <ul className="space-y-1">
      {visible.map((entry) => (
        <li className="flex items-baseline gap-1.5" key={entry.field}>
          <span className="shrink-0 font-medium">
            {humaniseFieldId(entry.field)}:
          </span>
          <span className={valueClass}>
            <ChangeDescription change={entry.change} />
          </span>
        </li>
      ))}
      {overflow > 0 ? (
        <li className={overflowClass}>and {overflow} more</li>
      ) : null}
    </ul>
  );
}

// ── Badge + tooltip ───────────────────────────────────────────────────────────

/**
 * The restart-required badge. When `restartDiff` is non-empty, shows a hover
 * tooltip with the itemised before→after diff (capped at {@link TOOLTIP_CAP}
 * entries) plus the auto-restart blurb. Renders as a non-interactive `<span>`
 * so it can safely be placed adjacent to (not inside) a `<button>` — it must
 * never be a descendant of an interactive element.
 */
export function RestartDiffBadge({
  autoRestartEnabled = false,
  restartDiff,
  className,
}: {
  autoRestartEnabled?: boolean;
  restartDiff: RestartDiffEntry[];
  className?: string;
}) {
  const badge = (
    <Badge className={cn("cursor-default gap-1", className)} variant="warning">
      <RefreshCw className="h-3 w-3" />
      Restart required
    </Badge>
  );

  if (restartDiff.length === 0) {
    // No diff data — render plain badge without tooltip.
    return badge;
  }

  return (
    <Tooltip>
      {/* asChild renders the trigger as the Badge's <span>, which is a valid
          non-interactive element. No nested button. */}
      <TooltipTrigger asChild>
        <Badge
          className={cn("cursor-default gap-1", className)}
          variant="warning"
          tabIndex={0}
          data-testid="restart-diff-badge"
        >
          <RefreshCw className="h-3 w-3" />
          Restart required
        </Badge>
      </TooltipTrigger>
      <TooltipContent className="max-w-72 text-xs" side="bottom">
        <p className="mb-1.5 font-semibold">Config changed since last start:</p>
        <DiffList cap={TOOLTIP_CAP} entries={restartDiff} />
        <p className="mt-1.5 text-primary-foreground/70">
          {autoRestartEnabled ? AUTO_RESTART_ON_BLURB : AUTO_RESTART_OFF_BLURB}
        </p>
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * Full uncapped diff list for the Runtime-tab banner. Renders inline; no
 * tooltip wrapper.
 */
export function RestartDiffList({
  restartDiff,
}: {
  restartDiff: RestartDiffEntry[];
}) {
  if (restartDiff.length === 0) return null;
  return (
    <div
      className="mt-2 text-xs text-muted-foreground"
      data-testid="restart-diff-list"
    >
      <DiffList entries={restartDiff} variant="inline" />
    </div>
  );
}
