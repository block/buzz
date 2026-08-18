import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  type WorkflowEditorRoute,
  WorkflowsScreen,
} from "@/features/workflows/ui/WorkflowsScreen";
import type { WorkflowEditorPane } from "@/features/workflows/ui/workflowEditorPane";

type WorkflowsRouteScreenProps = {
  editor?: WorkflowEditorRoute | null;
  onEditorPaneChange: (pane: WorkflowEditorPane) => void;
  selectedWorkflowId: string | null;
};

export function WorkflowsRouteScreen({
  editor = null,
  onEditorPaneChange,
  selectedWorkflowId,
}: WorkflowsRouteScreenProps) {
  const {
    closeWorkflowDetail,
    goDuplicateWorkflow,
    goEditWorkflow,
    goNewWorkflow,
    goWorkflow,
    goWorkflows,
  } = useAppNavigation();
  const closeEditor = React.useCallback(() => {
    if (editor?.hasOrigin) {
      window.history.back();
      return;
    }
    void goWorkflows({ replace: true });
  }, [editor?.hasOrigin, goWorkflows]);
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data ?? [];
  const memberChannels = channels.filter((channel) => channel.isMember);

  return (
    <WorkflowsScreen
      channels={memberChannels}
      editor={editor}
      onCloseEditor={closeEditor}
      onCloseWorkflow={closeWorkflowDetail}
      onCreateWorkflow={() => {
        void goNewWorkflow();
      }}
      onDuplicateWorkflow={(workflowId) => {
        void goDuplicateWorkflow(workflowId);
      }}
      onEditWorkflow={(workflowId) => {
        void goEditWorkflow(workflowId);
      }}
      onEditorPaneChange={onEditorPaneChange}
      onViewWorkflow={(workflowId) => {
        void goWorkflow(workflowId);
      }}
      selectedWorkflowId={selectedWorkflowId}
    />
  );
}
