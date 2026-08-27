import type * as React from "react";

import * as agentSessionSelection from "@/features/channels/ui/agentSessionSelection";
import type { ChannelAgentSessionAgent } from "@/features/channels/ui/useChannelAgentSessions";
import { getAgentSessionPanelPresentation } from "@/features/channels/lib/agentSessionPanelPresentation";
import { AgentActivityDrawer } from "@/features/channels/ui/AgentActivityDrawer";
import { AgentSessionThreadPanel } from "@/features/channels/ui/AgentSessionThreadPanel";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";

type ChannelAgentSessionSurfaceProps = {
  activeChannel: Channel;
  activeChannelId: string | null;
  activityAgents: BotActivityAgent[];
  agent: ChannelAgentSessionAgent;
  /** Render the panel inside the cover drawer instead of the split pane. */
  isCoverDrawer: boolean;
  isSinglePanelView: boolean;
  onBack?: () => void;
  onClose: () => void;
  openAgentSessionChannelId: string | null;
  profiles?: UserProfileLookup;
  useSplitAuxiliaryPane: boolean;
  widthPx: number;
  /**
   * Applies the split-pane presentation, including its resize affordances.
   * Supplied by `ChannelPane` because that pane owns the resize state; the
   * cover-drawer presentation is applied here.
   */
  wrapSplitPane: (panel: React.ReactNode) => React.ReactNode;
};

/**
 * The channel's agent activity surface: the session panel plus the channel
 * re-scoping its content and actions depend on.
 *
 * Split out of `ChannelPane` so the re-scoping rule below has one home and is
 * not another branch inside that component's auxiliary-surface chain. Which
 * presentation this lands in is decided upstream and applied through `wrap`.
 */
export function ChannelAgentSessionSurface({
  activeChannel,
  activeChannelId,
  activityAgents,
  agent,
  isCoverDrawer,
  isSinglePanelView,
  onBack,
  onClose,
  openAgentSessionChannelId,
  profiles,
  useSplitAuxiliaryPane,
  widthPx,
  wrapSplitPane,
}: ChannelAgentSessionSurfaceProps) {
  // When the panel was opened from a different channel than the currently
  // active one, re-scope it to the active channel so that both the
  // content/header AND channel-backed actions (e.g. Stop current turn) operate
  // on the same channel object.
  const effectiveAgentSessionChannelId =
    openAgentSessionChannelId && activeChannel.id !== openAgentSessionChannelId
      ? activeChannelId
      : openAgentSessionChannelId;
  const channel = effectiveAgentSessionChannelId
    ? effectiveAgentSessionChannelId === activeChannel.id
      ? activeChannel
      : null
    : agentSessionSelection.isAgentInActivityList({
          activityAgents,
          selectedAgent: agent,
        })
      ? activeChannel
      : null;

  const layoutProps = getAgentSessionPanelPresentation({
    isCoverDrawer,
    isSinglePanelView,
    useSplitAuxiliaryPane,
  });
  const panel = (
    <AgentSessionThreadPanel
      agent={agent}
      canInterruptTurn={agent.canInterruptTurn}
      channel={channel}
      channelId={effectiveAgentSessionChannelId}
      {...layoutProps}
      profiles={profiles}
      onBack={onBack}
      onClose={onClose}
      widthPx={widthPx}
    />
  );

  return isCoverDrawer ? (
    <AgentActivityDrawer
      channelName={activeChannel.name ?? "channel"}
      onClose={onClose}
    >
      {panel}
    </AgentActivityDrawer>
  ) : (
    wrapSplitPane(panel)
  );
}
