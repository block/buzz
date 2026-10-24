import * as React from "react";

import type { Workflow } from "@/shared/api/types";

type WorkflowEditorOverlayContextValue = {
  /** Opens the create editor over the current surface, optionally preselecting
   * a channel. */
  openNewWorkflow: ((channelId?: string) => void) | null;
  /** Opens an existing workflow over the current surface. Pass the workflow
   * when the caller already has it so the editor skips its loading state. */
  openWorkflow: ((workflowId: string, workflow?: Workflow) => void) | null;
};

const WorkflowEditorOverlayContext =
  React.createContext<WorkflowEditorOverlayContextValue>({
    openNewWorkflow: null,
    openWorkflow: null,
  });

/**
 * Lets any surface open the shared workflow editor as a modal above itself,
 * instead of navigating to the Workflows route first.
 */
export function WorkflowEditorOverlayProvider({
  children,
  onOpenNewWorkflow,
  onOpenWorkflow,
}: {
  children: React.ReactNode;
  onOpenNewWorkflow: (channelId?: string) => void;
  onOpenWorkflow: (workflowId: string, workflow?: Workflow) => void;
}) {
  const value = React.useMemo(
    () => ({
      openNewWorkflow: onOpenNewWorkflow,
      openWorkflow: onOpenWorkflow,
    }),
    [onOpenNewWorkflow, onOpenWorkflow],
  );

  return (
    <WorkflowEditorOverlayContext.Provider value={value}>
      {children}
    </WorkflowEditorOverlayContext.Provider>
  );
}

export function useWorkflowEditorOverlay() {
  return React.useContext(WorkflowEditorOverlayContext);
}
