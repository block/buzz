import { Link2, Plus } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { Project } from "@/features/projects/hooks";
import { useAddProjectChannelMutation } from "@/features/projects/useAddProjectChannel";
import { useCanManageProjectChannels } from "@/features/projects/useCanManageProjectChannels";
import { useSetProjectRelatedChannelMutation } from "@/features/projects/useProjectRelatedChannels";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { LinkProjectChannelDialog } from "./LinkProjectChannelDialog";

export function ProjectChannelManagement({
  channels,
  identityPubkey,
  project,
  relatedChannelIds,
}: {
  channels: Channel[];
  identityPubkey?: string;
  project: Project;
  relatedChannelIds: readonly string[];
}) {
  const { goChannel } = useAppNavigation();
  const [createOpen, setCreateOpen] = React.useState(false);
  const [linkOpen, setLinkOpen] = React.useState(false);
  const createMutation = useAddProjectChannelMutation();
  const linkMutation = useSetProjectRelatedChannelMutation(project);
  const { canCreate, canManage, ownerControlAgentPubkey } =
    useCanManageProjectChannels(project, channels, identityPubkey);
  const relatedIds = new Set(relatedChannelIds.map((id) => id.toLowerCase()));
  const normalizedHomeChannelId = project.projectChannelId?.toLowerCase();
  const eligibleChannels = channels.filter((channel) => {
    const channelId = channel.id.toLowerCase();
    return (
      channel.archivedAt === null &&
      channel.isMember &&
      channelId !== normalizedHomeChannelId &&
      !relatedIds.has(channelId)
    );
  });

  return (
    <>
      {canCreate ? (
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
      ) : null}
      {canManage ? (
        <LinkProjectChannelDialog
          channels={eligibleChannels}
          isPending={linkMutation.isPending}
          onLink={async (channelId) => {
            await linkMutation.mutateAsync({ channelId, linked: true });
            const linked = channels.find((channel) => channel.id === channelId);
            toast.success(
              linked ? `Channel "#${linked.name}" linked.` : "Channel linked.",
            );
          }}
          onOpenChange={setLinkOpen}
          open={linkOpen}
          projectName={project.name}
        />
      ) : null}
      {canManage ? (
        <Button
          aria-label="Link existing channel"
          className="h-6 w-6 shrink-0 rounded-md text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          data-testid="link-project-channel"
          onClick={() => setLinkOpen(true)}
          size="icon"
          title="Link existing channel"
          type="button"
          variant="ghost"
        >
          <Link2 className="h-4 w-4" />
        </Button>
      ) : null}
      <Button
        aria-label="Create channel"
        className="h-6 w-6 shrink-0 rounded-md text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        data-testid="add-project-channel"
        disabled={!canCreate}
        onClick={() => setCreateOpen(true)}
        size="icon"
        title={
          canCreate
            ? "Create channel"
            : "Only the project owner can create channels"
        }
        type="button"
        variant="ghost"
      >
        <Plus className="h-4 w-4" />
      </Button>
    </>
  );
}
