import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { useChannelModerationCapabilities } from "@/features/channels/ui/ChannelManagementModerationActions";
import type { Project } from "@/features/projects/projectModels";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";

export function resolveProjectChannelManagement(input: {
  activeHomeChannel: boolean;
  homeCanManage: boolean;
  projectIsLegacy: boolean;
  projectOwner: string;
  projectOwnerIsManaged: boolean;
  viewerIsProjectOwner: boolean;
  viewerOwnsProjectAgent: boolean;
}) {
  const canControlOwnerProject =
    input.viewerIsProjectOwner ||
    input.projectOwnerIsManaged ||
    input.viewerOwnsProjectAgent;
  return {
    canCreate: !input.projectIsLegacy && canControlOwnerProject,
    canManage:
      !input.projectIsLegacy &&
      (canControlOwnerProject ||
        (input.activeHomeChannel && input.homeCanManage)),
    ownerControlAgentPubkey:
      input.viewerOwnsProjectAgent &&
      !input.projectOwnerIsManaged &&
      !input.viewerIsProjectOwner
        ? input.projectOwner
        : undefined,
  };
}

export function useCanManageProjectChannels(
  project: Project,
  channels: Channel[],
  identityPubkey?: string,
) {
  const ownerProfileQuery = useUsersBatchQuery([project.owner], {
    enabled: Boolean(identityPubkey),
  });
  const projectOwnerProfile =
    ownerProfileQuery.data?.profiles[project.owner.toLowerCase()];
  const projectOwnerIsManaged = useIsManagedAgent(project.owner) === true;
  const viewerIsProjectOwner =
    identityPubkey?.toLowerCase() === project.owner.toLowerCase();
  const viewerOwnsProjectAgent = ownsAuthorAgent(
    projectOwnerProfile,
    identityPubkey,
  );
  const activeHomeChannel = channels.find(
    (channel) =>
      channel.id === project.projectChannelId && channel.archivedAt === null,
  );
  const homeMembersQuery = useChannelMembersQuery(
    activeHomeChannel?.id ?? null,
    Boolean(identityPubkey && activeHomeChannel),
  );
  const homeCapabilities = useChannelModerationCapabilities(
    homeMembersQuery.data,
    identityPubkey,
    Boolean(identityPubkey && activeHomeChannel),
  );
  return resolveProjectChannelManagement({
    activeHomeChannel: Boolean(activeHomeChannel),
    homeCanManage: homeCapabilities.canManageChannel,
    projectIsLegacy: project.legacy,
    projectOwner: project.owner,
    projectOwnerIsManaged,
    viewerIsProjectOwner,
    viewerOwnsProjectAgent,
  });
}
