import {
  isToolRunGroupActive,
  type ToolRunGroupStatus,
} from "./agentSessionToolRunStatus";

/**
 * Per-group presentation state for a collapsible tool-run group.
 *
 * Kept as a plain value with pure transitions so the expansion policy is
 * testable without a DOM, and so the React layer only has to store it.
 */
export type ToolRunGroupViewState = {
  /** Whether the group body is currently open. */
  expanded: boolean;
  /**
   * True once the reader has opened or closed anything in this group. Sticky:
   * after this flips, automatic policy never moves the group again. A reader
   * who deliberately opened a group to watch it must not have it closed under
   * them when the work finishes.
   */
  userInteracted: boolean;
  /** Whether low-signal internal steps are revealed. */
  showInternalSteps: boolean;
  /** Ids of individual child steps the reader expanded. */
  expandedItemIds: string[];
};

/**
 * Mount-aware initial state.
 *
 * A group that is already finished when it first mounts is history — it starts
 * collapsed so a replayed transcript reads as a narrative rather than a wall of
 * steps. A group that mounts while still running is live work being supervised,
 * so it starts open. Failed groups also start open: a failure the reader has to
 * click to discover is a failure the transcript hid.
 */
export function initialToolRunGroupViewState(
  status: ToolRunGroupStatus,
): ToolRunGroupViewState {
  return {
    expanded: isToolRunGroupActive(status) || status === "failed",
    userInteracted: false,
    showInternalSteps: false,
    expandedItemIds: [],
  };
}

/**
 * Apply the automatic policy after a status change.
 *
 * An untouched group that was active and has now completed successfully
 * auto-collapses, so the transcript keeps moving forward as the agent works.
 * Any of these keeps it open:
 *  - the reader interacted (sticky, forever)
 *  - the group ended in failure (conspicuous by default)
 *  - the group is still active
 */
export function reconcileToolRunGroupViewState(
  previous: ToolRunGroupViewState,
  previousStatus: ToolRunGroupStatus,
  nextStatus: ToolRunGroupStatus,
): ToolRunGroupViewState {
  if (previous.userInteracted) return previous;
  if (previousStatus === nextStatus) return previous;

  if (nextStatus === "failed") {
    return previous.expanded ? previous : { ...previous, expanded: true };
  }

  const settled =
    isToolRunGroupActive(previousStatus) && nextStatus === "completed";
  if (!settled) return previous;

  return {
    expanded: false,
    userInteracted: false,
    showInternalSteps: false,
    expandedItemIds: [],
  };
}

/** Reader toggled the whole group open or closed. */
export function setToolRunGroupExpanded(
  previous: ToolRunGroupViewState,
  expanded: boolean,
): ToolRunGroupViewState {
  return { ...previous, expanded, userInteracted: true };
}

/** Reader toggled one child step. Opening a step also opens the group. */
export function setToolRunGroupItemExpanded(
  previous: ToolRunGroupViewState,
  itemId: string,
  expanded: boolean,
): ToolRunGroupViewState {
  const current = new Set(previous.expandedItemIds);
  if (expanded) {
    current.add(itemId);
  } else {
    current.delete(itemId);
  }
  return {
    ...previous,
    expanded: expanded ? true : previous.expanded,
    userInteracted: true,
    expandedItemIds: [...current].sort(),
  };
}

/** Reader toggled the low-signal internal-step disclosure. */
export function setToolRunGroupInternalStepsShown(
  previous: ToolRunGroupViewState,
  showInternalSteps: boolean,
): ToolRunGroupViewState {
  return { ...previous, showInternalSteps, userInteracted: true };
}
