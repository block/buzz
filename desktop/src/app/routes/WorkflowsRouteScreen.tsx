import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { WorkflowsScreen } from "@/features/workflows/ui/WorkflowsScreen";

type WorkflowsRouteScreenProps = {
  selectedWorkflowId: string | null;
};

export function WorkflowsRouteScreen({
  selectedWorkflowId,
}: WorkflowsRouteScreenProps) {
  const { closeWorkflowDetail, goWorkflow } = useAppNavigation();
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
      onCloseWorkflow={closeWorkflowDetail}
      onSelectWorkflow={(workflowId) => {
        void goWorkflow(workflowId);
      }}
      selectedWorkflowId={selectedWorkflowId}
    />
  );
}
