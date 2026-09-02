import type { Project } from "@/features/projects/projectModels";
import { listProjectBoundChannels } from "@/features/projects/lib/projectRelatedChannels";
import type { Channel, ChannelMember } from "@/shared/api/types";

export function canManageProjectRelatedChannels(input: {
  homeChannelActive: boolean;
  homeChannelMembers: readonly ChannelMember[] | undefined;
  identityPubkey: string | undefined;
  project: Pick<Project, "legacy" | "owner" | "projectChannelId">;
}): boolean {
  const identity = input.identityPubkey?.toLowerCase();
  if (!identity || input.project.legacy) return false;
  if (identity === input.project.owner.toLowerCase()) return true;
  if (!input.project.projectChannelId || !input.homeChannelActive) return false;

  const role = input.homeChannelMembers?.find(
    (member) => member.pubkey.toLowerCase() === identity,
  )?.role;
  return role === "owner" || role === "admin";
}

export function listLinkableProjectChannels(
  project: Pick<
    Project,
    "projectChannelId" | "relatedChannelIds" | "repositories"
  >,
  channels: readonly Channel[],
): Channel[] {
  const boundIds = new Set(
    listProjectBoundChannels(project).map((binding) => binding.channelId),
  );
  return channels
    .filter(
      (channel) =>
        channel.isMember &&
        channel.channelType !== "dm" &&
        channel.archivedAt === null &&
        !boundIds.has(channel.id),
    )
    .sort((left, right) => left.name.localeCompare(right.name));
}
