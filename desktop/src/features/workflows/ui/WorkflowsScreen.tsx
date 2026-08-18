import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import type { WorkflowEditorPane } from "./workflowEditorPane";

const WorkflowsView = React.lazy(async () => {
  const module = await import("@/features/workflows/ui/WorkflowsView");
  return { default: module.WorkflowsView };
});

export type WorkflowEditorRoute =
  | {
      hasOrigin: boolean;
      mode: "create";
      pane: WorkflowEditorPane;
    }
  | {
      hasOrigin: boolean;
      mode: "duplicate" | "edit";
      pane: WorkflowEditorPane;
      workflowId: string;
    };

type WorkflowsScreenProps = {
  channels: Channel[];
  editor: WorkflowEditorRoute | null;
  onCloseEditor: () => void;
  onCloseWorkflow: () => void;
  onCreateWorkflow: () => void;
  onDuplicateWorkflow: (workflowId: string) => void;
  onEditWorkflow: (workflowId: string) => void;
  onEditorPaneChange: (pane: WorkflowEditorPane) => void;
  onViewWorkflow: (workflowId: string) => void;
  selectedWorkflowId: string | null;
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
