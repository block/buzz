import { Plus } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import type { Project } from "@/features/projects/hooks";
import { useAddProjectChannelMutation } from "@/features/projects/useAddProjectChannel";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import type { ChannelMember } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

export function ProjectChannelManagement({
  identityPubkey,
  project,
}: {
  identityPubkey?: string;
  project: Project;
}) {
  const { goChannel } = useAppNavigation();
  const [createOpen, setCreateOpen] = React.useState(false);
  const createMutation = useAddProjectChannelMutation();
  const access = useProjectChannelManagementAccess(project, identityPubkey);
  const canEdit = access.canManage;

  return (
    <>
      {canEdit ? (
        <CreateChannelDialog
          channelKind={createOpen ? "stream" : null}
          description="Add another stream to this project. A template can keep the same canvas and agents."
          isCreating={createMutation.isPending}
          onCreate={async (input) => {
            const result = await createMutation.mutateAsync({
              ...input,
              ownerControlAgentPubkey: access.ownerControlAgentPubkey,
              project,
              signAsManagedOwner: access.signAsManagedOwner,
            });
            toast.success(`Channel "#${result.channel.name}" created.`);
            await goChannel(result.channel.id);
          }}
          onOpenChange={setCreateOpen}
          testId="create-project-channel-dialog"
          title="Create a project channel"
        />
      ) : null}
      <Button
        aria-label="Add channel"
        className="h-6 w-6 shrink-0 rounded-md text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        data-testid="add-project-channel"
        disabled={!canEdit}
        onClick={() => setCreateOpen(true)}
        size="icon"
        title={
          canEdit
            ? "Add channel"
            : "Only the Project owner or a home-channel admin can add channels"
        }
        type="button"
        variant="ghost"
      >
        <Plus className="h-4 w-4" />
      </Button>
    </>
  );
}

export function useProjectChannelManagementAccess(
  project: Project,
  identityPubkey?: string,
): {
  canManage: boolean;
  ownerControlAgentPubkey?: string;
  signAsManagedOwner: boolean;
} {
  const homeMembersQuery = useChannelMembersQuery(project.projectChannelId);
  const ownerProfileQuery = useUsersBatchQuery([project.owner], {
    enabled: Boolean(identityPubkey),
  });
  const projectOwnerProfile =
    ownerProfileQuery.data?.profiles[project.owner.toLowerCase()];
  const projectOwnerIsManaged = useIsManagedAgent(project.owner) === true;
  const viewerOwnsProjectAgent = ownsAuthorAgent(
    projectOwnerProfile,
    identityPubkey,
  );
  const viewerIsProjectOwner =
    identityPubkey?.toLowerCase() === project.owner.toLowerCase();
  const viewerHomeRole = homeMembersQuery.data?.find(
    (member) => member.pubkey.toLowerCase() === identityPubkey?.toLowerCase(),
  )?.role;
  const viewerHasHomeRole =
    viewerHomeRole === "owner" || viewerHomeRole === "admin";
  const canManage = canManageProjectChannels(
    project,
    identityPubkey,
    viewerHomeRole,
    projectOwnerIsManaged || viewerOwnsProjectAgent,
  );
  return {
    canManage,
    ownerControlAgentPubkey:
      canManage &&
      !viewerIsProjectOwner &&
      !viewerHasHomeRole &&
      viewerOwnsProjectAgent &&
      !projectOwnerIsManaged
        ? project.owner
        : undefined,
    signAsManagedOwner:
      canManage &&
      !viewerIsProjectOwner &&
      !viewerHasHomeRole &&
      projectOwnerIsManaged,
  };
}

export function canManageProjectChannels(
  project: Project,
  identityPubkey?: string,
  viewerHomeRole?: ChannelMember["role"],
  viewerCanControlOwner = false,
): boolean {
  return (
    !project.legacy &&
    (identityPubkey?.toLowerCase() === project.owner.toLowerCase() ||
      viewerCanControlOwner ||
      viewerHomeRole === "owner" ||
      viewerHomeRole === "admin")
  );
}
