import * as React from "react";

import type { ToolRunGroupStatus } from "./agentSessionToolRunStatus";
import {
  initialToolRunGroupViewState,
  reconcileToolRunGroupViewState,
  type ToolRunGroupViewState,
} from "./agentSessionToolRunViewState";
import {
  readToolRunGroupViewState,
  writeToolRunGroupViewState,
} from "./agentSessionToolRunViewStore";

type ToolRunGroupViewStateEntry = {
  groupKey: string;
  status: ToolRunGroupStatus;
  state: ToolRunGroupViewState;
};

function createEntry(
  groupKey: string,
  status: ToolRunGroupStatus,
): ToolRunGroupViewStateEntry {
  return {
    groupKey,
    status,
    state:
      readToolRunGroupViewState(groupKey) ??
      initialToolRunGroupViewState(status),
  };
}

/**
 * Own one group's expansion state, restoring any remembered choice and applying
 * the automatic policy when the group's aggregate status changes.
 *
 * State is recomputed during render (rather than in an effect) so the very
 * first paint after a group finishes is already correct — an effect-based
 * collapse would flash the open group for a frame.
 */
export function useToolRunGroupViewState(
  groupKey: string,
  status: ToolRunGroupStatus,
): {
  state: ToolRunGroupViewState;
  update: (
    next: (previous: ToolRunGroupViewState) => ToolRunGroupViewState,
  ) => void;
} {
  const [entry, setEntry] = React.useState<ToolRunGroupViewStateEntry>(() =>
    createEntry(groupKey, status),
  );

  let current = entry;
  if (entry.groupKey !== groupKey) {
    current = createEntry(groupKey, status);
    setEntry(current);
  } else if (entry.status !== status) {
    current = {
      groupKey,
      status,
      state: reconcileToolRunGroupViewState(entry.state, entry.status, status),
    };
    setEntry(current);
  }

  const currentState = current.state;
  React.useEffect(() => {
    writeToolRunGroupViewState(groupKey, currentState);
  }, [groupKey, currentState]);

  const update = React.useCallback(
    (next: (previous: ToolRunGroupViewState) => ToolRunGroupViewState) => {
      setEntry((previous) => ({ ...previous, state: next(previous.state) }));
    },
    [],
  );

  return { state: currentState, update };
}
