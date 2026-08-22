import * as React from "react";
import { CircleAlert, FileDiff, FilePen, Loader } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import {
  ActivityRow,
  ActivityRowContent,
} from "./activityRenderClasses/ActivityRow";
import type {
  TranscriptToolRunChildSegment,
  TranscriptToolRunSummary,
} from "./agentSessionTranscriptGrouping";
import { getToolRunGroupKey } from "./agentSessionToolRunGroupKey";
import {
  getToolRunGroupStatus,
  isToolRunGroupActive,
} from "./agentSessionToolRunStatus";
import {
  collectToolRunArtifacts,
  partitionToolRunSteps,
} from "./agentSessionToolRunPartition";
import {
  setToolRunGroupExpanded,
  setToolRunGroupInternalStepsShown,
  setToolRunGroupItemExpanded,
} from "./agentSessionToolRunViewState";
import { useToolRunGroupViewState } from "./useToolRunGroupViewState";
import { useToolRunGroupKey } from "./useTranscriptToolRunGroupKeys";
import { formatTranscriptTimestampTitle } from "./agentSessionUtils";
import { TranscriptRowTimestamp } from "./TranscriptRowTimestamp";

/**
 * Status shown beside a collapsed group's label.
 *
 * Only non-clean outcomes render. A completed group says nothing — "done" is
 * the expected case and a badge on every finished group would train the reader
 * to ignore the badge that matters. In-flight work is exactly the state worth
 * interrupting for.
 *
 * The `failed` branch is a safety net that mirrors the failure-leaning fold in
 * `getToolRunGroupStatus`; no observer frame reaches it today because grouping
 * ejects failed calls into standalone rows (see that module's note). It is kept
 * deliberately plain — one word, no count — because an aggregate that cannot be
 * produced should not carry presentation detail nobody can verify. If grouping
 * ever admits failures, this says the true thing rather than nothing.
 */
function ToolRunGroupStatusBadge({
  status,
}: {
  status: ReturnType<typeof getToolRunGroupStatus>;
}) {
  if (status === "completed") return null;

  if (status === "failed") {
    return (
      <span
        className="inline-flex shrink-0 items-center gap-1 text-xs font-semibold text-status-deleted"
        data-testid="tool-run-group-status-failed"
      >
        <CircleAlert aria-hidden="true" className="h-3.5 w-3.5" />
        failed
      </span>
    );
  }

  return (
    <span
      className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground"
      data-testid="tool-run-group-status-running"
    >
      <Loader aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
      {status === "pending" ? "queued" : "running"}
    </span>
  );
}

/**
 * Files a group touched, rendered OUTSIDE its collapsible body.
 *
 * A file the agent edited is an outcome of the turn, not a detail of how the
 * steps happened to be batched — so it stays visible when the group is
 * collapsed. Otherwise the most consequential thing a group did would be the
 * thing a reader has to expand to discover.
 */
function ToolRunGroupArtifacts({
  artifacts,
}: {
  artifacts: ReturnType<typeof collectToolRunArtifacts>;
}) {
  if (artifacts.length === 0) return null;

  return (
    <div
      className="mt-0.5 flex flex-wrap items-center gap-1 pl-5"
      data-testid="tool-run-group-artifacts"
    >
      {artifacts.map((artifact) => (
        <span
          className={cn(
            "inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-border/60 px-1.5 py-0.5 text-xs",
            artifact.kind === "edit"
              ? "text-foreground"
              : "text-muted-foreground",
          )}
          key={`${artifact.kind}:${artifact.path}`}
          title={artifact.path}
        >
          {artifact.kind === "edit" ? (
            <FilePen aria-hidden="true" className="h-3 w-3 shrink-0" />
          ) : (
            <FileDiff aria-hidden="true" className="h-3 w-3 shrink-0" />
          )}
          <span className="truncate">{artifact.filename}</span>
        </span>
      ))}
    </div>
  );
}

export type TranscriptToolRunGroupProps = {
  summary: TranscriptToolRunSummary;
  /**
   * Group label, already split/animated by the list. Passed as a node (rather
   * than a string plus stats) so the group card owns none of the label's
   * animation policy.
   */
  label: React.ReactNode;
  showTimestamp: boolean;
  /** Renders one child segment. Passed in to avoid a list↔group import cycle. */
  renderChild: (
    child: TranscriptToolRunChildSegment,
    expansion?: {
      expanded: boolean;
      onExpansionChange: (expanded: boolean) => void;
    },
  ) => React.ReactNode;
};

/**
 * A collapsible group of adjacent tool calls.
 *
 * Owns three behaviors the plain `ActivityRow` cannot express:
 *  - a failure-leaning aggregate status, so a collapsed group never reads as
 *    fine while something inside it failed or is still running;
 *  - mount-aware sticky expansion keyed by a durable identity, so live work is
 *    open while it runs, history arrives collapsed, and a reader's own choice is
 *    never reverted by re-grouping or by the group finishing;
 *  - artifacts and status rendered outside the collapsible body, so collapsing
 *    hides the steps without hiding the outcome.
 */
export function TranscriptToolRunGroup({
  label,
  renderChild,
  showTimestamp,
  summary,
}: TranscriptToolRunGroupProps) {
  const status = getToolRunGroupStatus(summary.items);
  const anchorKey = getToolRunGroupKey(summary);
  const groupKey = useToolRunGroupKey(summary.id, anchorKey);
  const { state, update } = useToolRunGroupViewState(groupKey, status);

  const artifacts = React.useMemo(
    () => collectToolRunArtifacts(summary.items),
    [summary.items],
  );

  // Children are the mixed burst's own segments when present, otherwise the
  // group's leaf items.
  const childSegments = React.useMemo<TranscriptToolRunChildSegment[]>(
    () =>
      summary.segments ??
      summary.items.map((item) => ({ kind: "item" as const, item })),
    [summary.items, summary.segments],
  );

  const { hiddenItems, primaryItems } = React.useMemo(
    () =>
      partitionToolRunSteps(
        childSegments.flatMap((child) =>
          child.kind === "item" ? [child.item] : child.summary.items,
        ),
        state.expandedItemIds,
      ),
    [childSegments, state.expandedItemIds],
  );

  const hiddenIds = React.useMemo(
    () => new Set(hiddenItems.map((item) => item.id)),
    [hiddenItems],
  );
  const visibleChildren = state.showInternalSteps
    ? childSegments
    : childSegments.filter(
        (child) => child.kind !== "item" || !hiddenIds.has(child.item.id),
      );

  return (
    <>
      <ActivityRow
        className="flex flex-col gap-0.5"
        onOpenChange={(open) =>
          update((previous) => setToolRunGroupExpanded(previous, open))
        }
        open={state.expanded}
        openToneScope="summary"
        testId="transcript-same-kind-summary"
        title={formatTranscriptTimestampTitle(summary.timestamp)}
      >
        {label}
        <ToolRunGroupStatusBadge status={status} />
        <ActivityRowContent className="flex flex-col gap-0.5">
          {visibleChildren.map((child) =>
            renderChild(
              child,
              child.kind === "item" && child.item.type === "tool"
                ? {
                    expanded: state.expandedItemIds.includes(child.item.id),
                    onExpansionChange: (expanded) =>
                      update((previous) =>
                        setToolRunGroupItemExpanded(
                          previous,
                          child.item.id,
                          expanded,
                        ),
                      ),
                  }
                : undefined,
            ),
          )}
          {hiddenItems.length > 0 ? (
            <button
              className="mt-0.5 self-start text-xs text-muted-foreground underline decoration-dotted hover:text-foreground"
              data-testid="tool-run-group-internal-toggle"
              onClick={() =>
                update((previous) =>
                  setToolRunGroupInternalStepsShown(
                    previous,
                    !previous.showInternalSteps,
                  ),
                )
              }
              type="button"
            >
              {state.showInternalSteps
                ? "Hide routine steps"
                : `Show ${hiddenItems.length} routine steps`}
            </button>
          ) : null}
          {primaryItems.length === 0 && visibleChildren.length === 0 ? (
            <p className="text-xs text-muted-foreground">No steps to show.</p>
          ) : null}
        </ActivityRowContent>
      </ActivityRow>
      <ToolRunGroupArtifacts artifacts={artifacts} />
      {showTimestamp ? (
        <TranscriptRowTimestamp timestamp={summary.timestamp} />
      ) : null}
    </>
  );
}

export { isToolRunGroupActive };
