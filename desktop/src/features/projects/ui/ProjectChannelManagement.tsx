import { Plus } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import type { Project } from "@/features/projects/hooks";
import { useAddProjectChannelMutation } from "@/features/projects/useAddProjectChannel";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
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
  const canEdit =
    !project.legacy &&
    (viewerIsProjectOwner || projectOwnerIsManaged || viewerOwnsProjectAgent);
  const ownerControlAgentPubkey =
    viewerOwnsProjectAgent && !projectOwnerIsManaged && !viewerIsProjectOwner
      ? project.owner
      : undefined;

  if (!canEdit) return null;

  return (
    <>
      <CreateChannelDialog
        channelKind={createOpen ? "stream" : null}
        description="Add another stream to this project. A template can keep the same canvas and agents."
        isCreating={createMutation.isPending}
        onCreate={async (input) => {
          const result = await createMutation.mutateAsync({
            ...input,
            ownerControlAgentPubkey,
            project,
          });
          toast.success(`Channel "#${result.channel.name}" created.`);
          await goChannel(result.channel.id);
        }}
        onOpenChange={setCreateOpen}
        testId="create-project-channel-dialog"
        title="Create a project channel"
      />
      <Button
        aria-label="Add channel"
        className="h-6 w-6 shrink-0 rounded-md text-muted-foreground hover:bg-muted/70 hover:text-foreground"
        data-testid="add-project-channel"
        onClick={() => setCreateOpen(true)}
        size="icon"
        type="button"
        variant="ghost"
      >
        <Plus className="h-4 w-4" />
      </Button>
    </>
  );
}
