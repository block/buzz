import * as React from "react";

import type {
  WorkflowEditorCreateTarget,
  WorkflowEditorWorkflowTarget,
} from "@/features/workflows/ui/WorkflowEditorHost";
import type { Channel } from "@/shared/api/types";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import type { WorkflowEditorPane } from "./workflowEditorPane";

const WorkflowsView = React.lazy(async () => {
  const module = await import("@/features/workflows/ui/WorkflowsView");
  return { default: module.WorkflowsView };
});

/** URL-addressable editor target: a shared editor target plus the history
 * origin the Workflows route needs to close back to where the user came from. */
export type WorkflowEditorRoute =
  | (WorkflowEditorCreateTarget & { hasOrigin: boolean })
  | (WorkflowEditorWorkflowTarget & { hasOrigin: boolean });

type WorkflowsScreenProps = {
  channels: Channel[];
  editor: WorkflowEditorRoute | null;
  onCloseEditor: () => void;
  onCreateWorkflow: () => void;
  onDuplicateWorkflow: (workflowId: string) => void;
  onEditWorkflow: (workflowId: string) => void;
  onViewWorkflow: (workflowId: string) => void;
  onEditorPaneChange: (pane: WorkflowEditorPane) => void;
};

export function WorkflowsScreen(props: WorkflowsScreenProps) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
        <WorkflowsView {...props} />
      </React.Suspense>
    </div>
  );
}
