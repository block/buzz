import type { Project } from "@/features/projects/projectModels";
import { canManageProjectChannels } from "@/features/projects/ui/ProjectChannelManagement";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ProjectChannelRequestApproval = {
  ownerControlAgentPubkey?: string;
};

/** Resolves authority and signer for a trusted managed-agent channel request. */
export function projectChannelRequestApproval(
  project: Project,
  identityPubkey: string | undefined,
  viewerHomeRole: ChannelMember["role"] | undefined,
  sourceAgentPubkey: string,
): ProjectChannelRequestApproval | null {
  if (project.legacy) return null;
  const sourceIsProjectOwner =
    normalizePubkey(sourceAgentPubkey) === normalizePubkey(project.owner);
  if (
    !sourceIsProjectOwner &&
    !canManageProjectChannels(project, identityPubkey, viewerHomeRole)
  ) {
    return null;
  }
  return sourceIsProjectOwner
    ? { ownerControlAgentPubkey: sourceAgentPubkey }
    : {};
}
