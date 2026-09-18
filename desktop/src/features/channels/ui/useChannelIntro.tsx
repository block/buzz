import * as React from "react";
import { Bot, FolderPlus, Plus, Sparkles, UserPlus } from "lucide-react";

import {
  getChannelIntroDescription,
  getChannelIntroKind,
} from "@/features/channels/ui/ChannelPane.helpers";
import {
  isWelcomeChannel,
  isWelcomeExperienceChannel,
} from "@/features/onboarding/welcome";
import { useIsProjectHomeChannel } from "@/features/projects/lib/projectHomeChannel";
import { ProjectChannelIcon } from "@/features/projects/ui/ProjectChannelIcon";
import type { Channel } from "@/shared/api/types";
import { HashSearch } from "@/shared/ui/icons";

type ChannelIntroAction = {
  description?: string;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  testId?: string;
};

/**
 * Builds the empty-channel intro block (heading, description, action cards)
 * for the channel timeline. The Welcome channel gets its onboarding trio
 * (browse / create channel / create agent); other channels get contextual
 * member actions.
 */
export function useChannelIntro({
  activeChannel,
  onAddAgent,
  onAddFiles,
  onBrowseChannels,
  onCreateChannel,
  onOpenMembers,
  onWelcomeAddAgent,
}: {
  activeChannel: Channel | null;
  onAddAgent?: (options?: { beforeSend?: () => void }) => void;
  onAddFiles?: () => void;
  onBrowseChannels?: () => void;
  onCreateChannel?: () => void;
  onOpenMembers?: () => void;
  onWelcomeAddAgent?: () => void;
}) {
  const projectHome = useIsProjectHomeChannel(activeChannel?.id);

  return React.useMemo(() => {
    if (!activeChannel || activeChannel.channelType === "dm") {
      return null;
    }

    const actions: ChannelIntroAction[] = [];
    if (isWelcomeExperienceChannel(activeChannel)) {
      if (onBrowseChannels) {
        actions.push({
          icon: <HashSearch aria-hidden className="h-6 w-6" />,
          label: "Browse channels",
          onClick: onBrowseChannels,
          testId: "welcome-intro-action-browse-channels",
        });
      }

      if (onCreateChannel) {
        actions.push({
          icon: <Plus aria-hidden className="h-6 w-6" />,
          label: "Create a channel",
          onClick: onCreateChannel,
          testId: "welcome-intro-action-create-channel",
        });
      }

      if (onWelcomeAddAgent) {
        actions.push({
          icon: <Bot aria-hidden className="h-6 w-6" />,
          label: "Create an agent",
          onClick: onWelcomeAddAgent,
          testId: "welcome-intro-action-create-agent",
        });
      }

      return {
        actions,
        channelKindLabel: isWelcomeChannel(activeChannel)
          ? "private welcome channel"
          : getChannelIntroKind(activeChannel, projectHome),
        channelName: activeChannel.name,
        description: isWelcomeChannel(activeChannel)
          ? null
          : getChannelIntroDescription(activeChannel),
        icon: <Sparkles aria-hidden className="h-7 w-7" />,
      };
    }

    if (!activeChannel.archivedAt && activeChannel.isMember) {
      if (onAddFiles) {
        actions.push({
          description: "Add a repo.",
          icon: <FolderPlus aria-hidden className="h-5 w-5" />,
          label: "Add files",
          onClick: onAddFiles,
          testId: "channel-intro-action-add-files",
        });
      }

      if (onAddAgent) {
        actions.push({
          description: "Add an agent here.",
          icon: <Bot aria-hidden className="h-5 w-5" />,
          label: "Add agent",
          onClick: onAddAgent,
          testId: "channel-intro-action-create-agent",
        });
      }

      if (onOpenMembers) {
        actions.push({
          description: "Invite members.",
          icon: <UserPlus aria-hidden className="h-5 w-5" />,
          label: "Add people",
          onClick: onOpenMembers,
          testId: "channel-intro-action-add-people",
        });
      }
    }

    return {
      actions,
      channelKindLabel: getChannelIntroKind(activeChannel, projectHome),
      channelName: activeChannel.name,
      description: getChannelIntroDescription(activeChannel),
      hideBeginning: projectHome,
      icon: projectHome ? (
        <ProjectChannelIcon className="h-7 w-7" />
      ) : undefined,
    };
  }, [
    activeChannel,
    onAddAgent,
    onAddFiles,
    onBrowseChannels,
    onCreateChannel,
    onOpenMembers,
    onWelcomeAddAgent,
    projectHome,
  ]);
}
