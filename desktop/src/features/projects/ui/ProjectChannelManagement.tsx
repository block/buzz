import { Plus } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { Project } from "@/features/projects/hooks";
import { listLinkableProjectChannels } from "@/features/projects/lib/projectRelatedChannelAccess";
import { useChangeProjectRelatedChannelsMutation } from "@/features/projects/useChangeProjectRelatedChannels";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { AddProjectChannelDialog } from "./AddProjectChannelDialog";

export function ProjectChannelManagement({
  canManage,
  channels,
  project,
}: {
  canManage: boolean;
  channels: Channel[];
  project: Project;
}) {
  const [addOpen, setAddOpen] = React.useState(false);
  const changeMutation = useChangeProjectRelatedChannelsMutation();
  const candidates = React.useMemo(
    () => listLinkableProjectChannels(project, channels),
    [channels, project],
  );

  return (
    <>
      {canManage ? (
        <AddProjectChannelDialog
          channels={candidates}
          isAdding={changeMutation.isPending}
          onAdd={async (channel) => {
            await changeMutation.mutateAsync({
              add: [channel.id],
              project,
            });
            toast.success(`Channel "#${channel.name}" added to project.`);
          }}
          onOpenChange={setAddOpen}
          open={addOpen}
          project={project}
        />
      ) : null}
      <Button
        aria-label="Add channel"
        className="h-6 w-6 shrink-0 rounded-md text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        data-testid="add-project-channel"
        disabled={!canManage}
        onClick={() => setAddOpen(true)}
        size="icon"
        title={
          canManage
            ? "Add channel"
            : "Only the project owner or a home-channel admin can add channels"
        }
        type="button"
        variant="ghost"
      >
        <Plus className="h-4 w-4" />
      </Button>
    </>
  );
}
