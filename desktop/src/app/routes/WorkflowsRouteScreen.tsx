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
};

export function WorkflowsRouteScreen({
  editor = null,
  onEditorPaneChange,
}: WorkflowsRouteScreenProps) {
  const {
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
  // Member channels plus open channels the owner hasn't joined yet — open
  // channels are readable without membership (see channelDescription.ts),
  // so a workflow living there is fully visible and must not be silently
  // dropped just because isMember is false. WorkflowsView narrows further
  // for membership-gated actions (e.g. the create-workflow channel picker).
  const visibleChannels = channels.filter(
    (channel) => channel.isMember || channel.visibility === "open",
  );

  return (
    <WorkflowsScreen
      channels={visibleChannels}
      editor={editor}
      onCloseEditor={closeEditor}
      onCreateWorkflow={() => {
        void goNewWorkflow();
      }}
      onDuplicateWorkflow={(workflowId) => {
        void goDuplicateWorkflow(workflowId);
      }}
      onEditWorkflow={(workflowId) => {
        void goEditWorkflow(workflowId);
      }}
      onViewWorkflow={(workflowId) => {
        void goWorkflow(workflowId);
      }}
      onEditorPaneChange={onEditorPaneChange}
    />
  );
}
