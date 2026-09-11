import type { TranscriptItem } from "./agentSessionTypes";

/**
 * Aggregate status for a collapsed group of transcript activity.
 *
 * The ordering below is deliberately failure-leaning: a collapsed parent must
 * never present a group as finished-and-fine while one of its children failed
 * or is still running. Reading the aggregate is the whole point of collapsing,
 * so the aggregate is the one place we refuse to round in the optimistic
 * direction.
 *
 * `"failed"` is currently a SAFETY NET, not a state real frames reach. Grouping
 * ejects failures (`isGroupingEligible` in `agentSessionTranscriptGrouping`
 * rejects `isError`, and every failed item is error-flagged — `tool_call`
 * frames pass `isError: false` and `tool_call_update` derives it from
 * `status === "failed"`), so a failed call always breaks out as its own row.
 * That ejection, not this aggregate, is what keeps failures conspicuous today.
 * The fold still handles failure so that widening eligibility later cannot
 * silently hide a failure behind a collapsed summary.
 */
export type ToolRunGroupStatus =
  | "failed"
  | "executing"
  | "pending"
  | "completed";

/** Precedence order applied by {@link getToolRunGroupStatus} (worst first). */
const STATUS_PRECEDENCE: ToolRunGroupStatus[] = [
  "failed",
  "executing",
  "pending",
  "completed",
];

function statusForItem(item: TranscriptItem): ToolRunGroupStatus {
  if (item.type === "tool") {
    if (item.isError || item.status === "failed") return "failed";
    if (item.status === "executing") return "executing";
    if (item.status === "pending") return "pending";
    return "completed";
  }

  // Non-tool rows can still carry failure (a lifecycle error reclassified into
  // the group's span). Everything else is inert for aggregation purposes.
  return item.renderClass === "error" ? "failed" : "completed";
}

/**
 * Fold a group's children into one status, leaning toward the worst outcome.
 *
 * `failed` beats `executing` beats `pending` beats `completed`. An empty group
 * reports `completed` — there is nothing outstanding to warn about.
 */
export function getToolRunGroupStatus(
  items: readonly TranscriptItem[],
): ToolRunGroupStatus {
  let worstIndex = STATUS_PRECEDENCE.length - 1;
  for (const item of items) {
    const index = STATUS_PRECEDENCE.indexOf(statusForItem(item));
    if (index < worstIndex) {
      worstIndex = index;
    }
    if (worstIndex === 0) break;
  }
  return STATUS_PRECEDENCE[worstIndex];
}

/** True while a group still has work outstanding. */
export function isToolRunGroupActive(status: ToolRunGroupStatus): boolean {
  return status === "executing" || status === "pending";
}
