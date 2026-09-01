import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useChannelMembersQuery,
  useChannelsQuery,
} from "@/features/channels/hooks";
import type { Project } from "@/features/projects/hooks";
import { useAddProjectChannelMutation } from "@/features/projects/useAddProjectChannel";
import { useAddProjectRepositoryMutation } from "@/features/projects/useAddProjectRepository";
import { AddProjectRepositoryDialog } from "@/features/projects/ui/AddProjectRepositoryDialog";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import { canManageProjectChannels } from "@/features/projects/ui/ProjectChannelManagement";

export function projectsAvailableForChannelCreation(
  projects: Project[],
  editableProjectIds: ReadonlySet<string>,
): Project[] {
  return projects.filter(
    (project) =>
      !project.legacy &&
      (editableProjectIds.has(project.id) || Boolean(project.projectChannelId)),
  );
}

export function canOpenProjectChannelDialog(
  projects: Project[],
  editableProjects: Project[],
): boolean {
  return (
    projectsAvailableForChannelCreation(
      projects,
      new Set(editableProjects.map((project) => project.id)),
    ).length > 0
  );
}

export function shouldLoadProjectHomeRoster(
  project: Project | undefined,
  channelOpen: boolean,
  viewerCanControlOwner: boolean,
  identityPubkey?: string,
): boolean {
  return Boolean(
    channelOpen &&
      project?.projectChannelId &&
      !viewerCanControlOwner &&
      project.owner.toLowerCase() !== identityPubkey?.toLowerCase(),
  );
}

export function ProjectsCategoryCreateDialogs({
  channelCandidateProjects,
  channelOpen,
  editableProjects,
  identityPubkey,
  onChannelOpenChange,
  onRepositoryOpenChange,
  ownerControlAgentPubkeyFor,
  repositoryOpen,
}: {
  channelCandidateProjects: Project[];
  channelOpen: boolean;
  editableProjects: Project[];
  identityPubkey?: string;
  onChannelOpenChange: (open: boolean) => void;
  onRepositoryOpenChange: (open: boolean) => void;
  ownerControlAgentPubkeyFor: (project: Project) => string | undefined;
  repositoryOpen: boolean;
}) {
  const { goChannel, goProject } = useAppNavigation();
  const editableProjectIds = React.useMemo(
    () => new Set(editableProjects.map((project) => project.id)),
    [editableProjects],
  );
  const channelProjects = projectsAvailableForChannelCreation(
    channelCandidateProjects,
    editableProjectIds,
  );
  const [channelProjectId, setChannelProjectId] = React.useState("");
  const channelProject =
    channelProjects.find((project) => project.id === channelProjectId) ??
    channelProjects[0];
  const viewerCanControlChannelOwner = Boolean(
    channelProject && editableProjectIds.has(channelProject.id),
  );
  const homeMembersQuery = useChannelMembersQuery(
    channelProject?.projectChannelId ?? null,
    shouldLoadProjectHomeRoster(
      channelProject,
      channelOpen,
      viewerCanControlChannelOwner,
      identityPubkey,
    ),
  );
  const channelViewerRole = homeMembersQuery.data?.find(
    (member) => member.pubkey.toLowerCase() === identityPubkey?.toLowerCase(),
  )?.role;
  const canManageChannelProject = Boolean(
    channelProject &&
      canManageProjectChannels(
        channelProject,
        identityPubkey,
        channelViewerRole,
        viewerCanControlChannelOwner,
      ),
  );
  const channelOwnerControlAgentPubkey = channelProject
    ? ownerControlAgentPubkeyFor(channelProject)
    : undefined;
  const channelSignAsManagedOwner = Boolean(
    channelProject &&
      viewerCanControlChannelOwner &&
      identityPubkey?.toLowerCase() !== channelProject.owner.toLowerCase() &&
      channelViewerRole !== "owner" &&
      channelViewerRole !== "admin" &&
      !channelOwnerControlAgentPubkey,
  );
  const createChannelMutation = useAddProjectChannelMutation();
  const createRepositoryMutation = useAddProjectRepositoryMutation();
  const channelsQuery = useChannelsQuery({ enabled: repositoryOpen });
  const repositoryAccessChannels = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) =>
          channel.isMember &&
          !channel.archivedAt &&
          channel.channelType !== "dm",
      ),
    [channelsQuery.data],
  );

  return (
    <>
      <CreateChannelDialog
        channelKind={channelOpen ? "stream" : null}
        description={
          channelProject
            ? `Add another stream to ${channelProject.name}.`
            : "Choose a project for this channel."
        }
        isCreating={createChannelMutation.isPending}
        submitEnabled={canManageChannelProject}
        onCreate={async (input) => {
          if (!channelProject) throw new Error("Choose a project.");
          const result = await createChannelMutation.mutateAsync({
            ...input,
            ownerControlAgentPubkey: channelOwnerControlAgentPubkey,
            project: channelProject,
            signAsManagedOwner: channelSignAsManagedOwner,
          });
          toast.success(`Channel "#${result.channel.name}" created.`);
          await goChannel(result.channel.id);
        }}
        onOpenChange={onChannelOpenChange}
        testId="create-project-channel-dialog"
        title="Create a project channel"
      >
        <label className="block space-y-1.5 text-sm font-medium">
          <span>Project</span>
          <select
            className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
            data-testid="create-project-channel-project"
            disabled={createChannelMutation.isPending}
            onChange={(event) => setChannelProjectId(event.target.value)}
            value={channelProject?.id ?? ""}
          >
            {channelProjects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
      </CreateChannelDialog>
      <AddProjectRepositoryDialog
        channels={repositoryAccessChannels}
        isCreating={createRepositoryMutation.isPending}
        onAdd={async (input) => {
          const result = await createRepositoryMutation.mutateAsync({
            ...input,
            ownerControlAgentPubkey: ownerControlAgentPubkeyFor(input.project),
          });
          toast.success(`Repository "${result.repository.name}" created.`);
          await goProject(input.project.id, {
            repositoryId: result.repository.id,
          });
        }}
        onOpenChange={onRepositoryOpenChange}
        open={repositoryOpen}
        projects={editableProjects}
      />
    </>
  );
}
