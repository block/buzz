import * as React from "react";
import { Check, FolderPlus, Link, Plus, ShieldCheck } from "lucide-react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { Project, Repository } from "@/features/projects/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { useAddProjectRepositoryMutation } from "@/features/projects/useAddProjectRepository";
import { useAttachProjectRepositoryMutation } from "@/features/projects/useAttachProjectRepository";
import { useBindProjectRepositoryChannelMutation } from "@/features/projects/useBindProjectRepositoryChannel";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { AddProjectRepositoryDialog } from "./AddProjectRepositoryDialog";
import { AttachProjectRepositoryDialog } from "./AttachProjectRepositoryDialog";

export function ProjectRepositoryManagement({
  compact = false,
  createOpen: createOpenProp,
  hideTriggers = false,
  identityPubkey,
  onChange,
  onCreateOpenChange,
  project,
  projects,
  repository,
}: {
  compact?: boolean;
  createOpen?: boolean;
  hideTriggers?: boolean;
  identityPubkey?: string;
  onChange: (repositoryId: string) => void;
  onCreateOpenChange?: (open: boolean) => void;
  project: Project;
  projects: Project[];
  repository?: Repository | null;
}) {
  const [uncontrolledCreateOpen, setUncontrolledCreateOpen] =
    React.useState(false);
  const createOpen = createOpenProp ?? uncontrolledCreateOpen;
  const setCreateOpen = onCreateOpenChange ?? setUncontrolledCreateOpen;
  const [attachOpen, setAttachOpen] = React.useState(false);
  const channelsQuery = useChannelsQuery();
  const createMutation = useAddProjectRepositoryMutation();
  const attachMutation = useAttachProjectRepositoryMutation();
  const repairMutation = useBindProjectRepositoryChannelMutation();
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
    viewerIsProjectOwner || projectOwnerIsManaged || viewerOwnsProjectAgent;
  const ownerControlAgentPubkey =
    viewerOwnsProjectAgent && !projectOwnerIsManaged && !viewerIsProjectOwner
      ? project.owner
      : undefined;
  const accessChannels = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) =>
          channel.isMember &&
          !channel.archivedAt &&
          channel.channelType !== "dm",
      ),
    [channelsQuery.data],
  );
  const inheritedChannelId = [
    repository?.channelId,
    project.projectChannelId,
    project.repositories.find(
      (candidate) => candidate.id !== repository?.id && candidate.channelId,
    )?.channelId,
  ].find(
    (candidate) =>
      candidate && accessChannels.some((channel) => channel.id === candidate),
  );
  const canManageAccess =
    Boolean(repository) &&
    accessChannels.length > 0 &&
    identityPubkey?.toLowerCase() === repository?.owner.toLowerCase();
  const attachCandidates = React.useMemo(() => {
    const currentAddresses = new Set(project.repositoryAddresses);
    const candidates = new Map<string, Repository>();
    for (const candidateProject of projects) {
      for (const candidate of candidateProject.repositories) {
        if (!currentAddresses.has(candidate.repoAddress)) {
          candidates.set(candidate.repoAddress, candidate);
        }
      }
    }
    return [...candidates.values()].sort((left, right) =>
      left.name.localeCompare(right.name),
    );
  }, [project.repositoryAddresses, projects]);

  return (
    <>
      <AddProjectRepositoryDialog
        accessChannelId={inheritedChannelId ?? undefined}
        channels={accessChannels}
        isCreating={createMutation.isPending}
        onAdd={async (input) => {
          const result = await createMutation.mutateAsync({
            ...input,
            ownerControlAgentPubkey,
          });
          onChange(result.repository.id);
          toast.success(`Repository "${result.repository.name}" created.`);
        }}
        onOpenChange={setCreateOpen}
        open={createOpen}
        project={project}
      />
      <AttachProjectRepositoryDialog
        isAttaching={attachMutation.isPending}
        onAttach={async (candidate) => {
          const result = await attachMutation.mutateAsync({
            ownerControlAgentPubkey,
            project,
            repository: candidate,
          });
          onChange(result.repository.id);
          toast.success(`Repository "${result.repository.name}" added.`);
        }}
        onOpenChange={setAttachOpen}
        open={attachOpen}
        project={project}
        repositories={attachCandidates}
      />
      {!hideTriggers ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Add repository"
              className={
                compact
                  ? "h-6 w-6 shrink-0 rounded-md text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
                  : "h-7 shrink-0 gap-1.5 rounded-md"
              }
              data-testid="add-project-repository"
              disabled={!canEdit}
              size={compact ? "icon" : "sm"}
              title={
                canEdit
                  ? "Add repository"
                  : "Only the project owner can add repositories"
              }
              type="button"
              variant={compact ? "ghost" : "outline"}
            >
              <Plus className={compact ? "h-4 w-4" : "h-3.5 w-3.5"} />
              {compact ? null : "Add"}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuItem
              data-testid="create-project-repository"
              onSelect={() => setCreateOpen(true)}
            >
              <FolderPlus className="h-4 w-4" />
              Create new repository
            </DropdownMenuItem>
            <DropdownMenuItem
              data-testid="attach-project-repository"
              onSelect={() => setAttachOpen(true)}
            >
              <Link className="h-4 w-4" />
              Select existing repository
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
      {canManageAccess ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Manage repository access"
              className={
                compact
                  ? "h-6 w-6 shrink-0 rounded-md text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
                  : "h-7 shrink-0 gap-1.5 rounded-md"
              }
              disabled={repairMutation.isPending}
              size={compact ? "icon" : "sm"}
              type="button"
              variant={compact ? "ghost" : "outline"}
            >
              <ShieldCheck className={compact ? "h-4 w-4" : "h-3.5 w-3.5"} />
              {compact
                ? null
                : repairMutation.isPending
                  ? "Updating…"
                  : "Access"}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-56">
            <DropdownMenuLabel>Repository access channel</DropdownMenuLabel>
            {accessChannels.map((channel) => (
              <DropdownMenuItem
                className="justify-between gap-4"
                key={channel.id}
                onSelect={() => {
                  if (channel.id === repository?.channelId) return;
                  if (!repository) return;
                  void repairMutation
                    .mutateAsync({
                      channelId: channel.id,
                      repository,
                    })
                    .then(() => {
                      toast.success(
                        `Repository access set to #${channel.name}.`,
                      );
                    })
                    .catch((error: unknown) => {
                      toast.error(
                        error instanceof Error
                          ? error.message
                          : "Failed to update repository access.",
                      );
                    });
                }}
              >
                <span className="min-w-0 truncate">#{channel.name}</span>
                {channel.id === repository?.channelId ? (
                  <Check className="h-4 w-4 shrink-0" />
                ) : null}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </>
  );
}
