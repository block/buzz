import { AgentSessionThreadPanel } from "@/features/channels/ui/AgentSessionThreadPanel";
import * as agentSessionSelection from "@/features/channels/ui/agentSessionSelection";
import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";

type AgentSessionAuxiliaryPanelProps = Pick<
  ChannelPaneProps,
  "openAgentSessionChannelId" | "profiles"
> & {
  activeChannel: NonNullable<ChannelPaneProps["activeChannel"]>;
  // ChannelPane defaults this prop, so it is always defined at this call site.
  activityAgents: NonNullable<ChannelPaneProps["activityAgents"]>;
  isSinglePanelView: boolean;
  onBack: ChannelPaneProps["onBackFromAgentSession"];
  onClose: ChannelPaneProps["onCloseAgentSession"];
  selectedAgent: NonNullable<
    ReturnType<typeof agentSessionSelection.resolveSelectedAgentSession>
  >;
  useSplitAuxiliaryPane: boolean;
  widthPx: number;
};

/**
 * Assembles the agent-session auxiliary pane for ChannelPane's pane chain.
 * Split out of ChannelPane.tsx to keep it under the per-file line cap.
 */
export function AgentSessionAuxiliaryPanel({
  activeChannel,
  activityAgents,
  isSinglePanelView,
  onBack,
  onClose,
  openAgentSessionChannelId,
  profiles,
  selectedAgent,
  useSplitAuxiliaryPane,
  widthPx,
}: AgentSessionAuxiliaryPanelProps) {
  // When the panel was opened from a different channel than the currently
  // active one, re-scope it to the active channel so that both the
  // content/header AND channel-backed actions (e.g. Stop current turn)
  // operate on the same channel object.
  const effectiveAgentSessionChannelId =
    openAgentSessionChannelId && activeChannel.id !== openAgentSessionChannelId
      ? activeChannel.id
      : openAgentSessionChannelId;
  return (
    <AgentSessionThreadPanel
      agent={selectedAgent}
      canInterruptTurn={selectedAgent.canInterruptTurn}
      channel={
        effectiveAgentSessionChannelId
          ? effectiveAgentSessionChannelId === activeChannel.id
            ? activeChannel
            : null
          : agentSessionSelection.isAgentInActivityList({
                activityAgents,
                selectedAgent,
              })
            ? activeChannel
            : null
      }
      channelId={effectiveAgentSessionChannelId}
      isSinglePanelView={useSplitAuxiliaryPane ? false : isSinglePanelView}
      layout={useSplitAuxiliaryPane ? "split" : "standalone"}
      transparentChrome={useSplitAuxiliaryPane}
      profiles={profiles}
      onBack={onBack}
      onClose={onClose}
      widthPx={widthPx}
    />
  );
}
