import { FolderGit2, Hash } from "lucide-react";
import type * as React from "react";

import type { Project } from "@/features/projects/hooks";
import { listProjectBoundChannels } from "@/features/projects/lib/projectRelatedChannels";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { ProjectChannelIcon } from "./ProjectChannelIcon";
import { ProjectChannelManagement } from "./ProjectChannelManagement";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";

const RESOURCE_ROW_CLASS =
  "h-11 w-full justify-start gap-3 rounded-none border-b border-border/60 px-1 text-left font-normal";

export function ProjectChannelResourcesView({
  channels,
  identityPubkey,
  onOpenChannel,
  onOpenRepository,
  onSelectChat,
  project,
  projects,
  relatedChannelIds,
  view,
}: {
  channels: Channel[];
  identityPubkey?: string;
  onOpenChannel: (channelId: string) => void;
  onOpenRepository: (repositoryId: string) => void;
  onSelectChat: () => void;
  project: Project;
  projects: Project[];
  relatedChannelIds?: readonly string[];
  view: "channels" | "repos";
}) {
  if (view === "channels") {
    const channelsById = new Map(
      channels.map((candidate) => [candidate.id, candidate]),
    );
    const boundChannels = listProjectBoundChannels({
      ...project,
      relatedChannelIds: relatedChannelIds ?? project.relatedChannelIds,
    }).flatMap((binding) => {
      const channel = channelsById.get(binding.channelId);
      return channel ? [{ ...binding, channel }] : [];
    });

    return (
      <ResourceViewShell
        action={
          <ProjectChannelManagement
            identityPubkey={identityPubkey}
            project={project}
          />
        }
        description="Streams grouped with this project"
        testId="project-channel-content-channels"
        title="Channels"
      >
        {boundChannels.length > 0 ? (
          boundChannels.map((binding) => {
            const home = binding.role === "home";
            const Icon = home ? ProjectChannelIcon : Hash;
            return (
              <Button
                className={RESOURCE_ROW_CLASS}
                data-testid={
                  home
                    ? "project-channel-resource-home-channel"
                    : `project-channel-resource-channel-${binding.channel.name}`
                }
                key={`${binding.role}:${binding.channel.id}`}
                onClick={() =>
                  home ? onSelectChat() : onOpenChannel(binding.channel.id)
                }
                type="button"
                variant="ghost"
              >
                <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">
                  {binding.channel.name}
                </span>
                {home ? (
                  <span className="text-xs text-muted-foreground">Home</span>
                ) : null}
              </Button>
            );
          })
        ) : (
          <EmptyResourceState>No channels are available.</EmptyResourceState>
        )}
      </ResourceViewShell>
    );
  }

  return (
    <ResourceViewShell
      action={
        <ProjectRepositoryManagement
          identityPubkey={identityPubkey}
          onChange={onOpenRepository}
          project={project}
          projects={projects}
          showAccessManagement={false}
        />
      }
      description="Repositories related to this channel"
      testId="project-channel-content-repos"
      title="Repos"
    >
      {project.repositories.length > 0 ? (
        project.repositories.map((repository) => (
          <Button
            className={RESOURCE_ROW_CLASS}
            data-testid={`project-channel-resource-repository-${repository.dtag}`}
            key={repository.id}
            onClick={() => onOpenRepository(repository.id)}
            type="button"
            variant="ghost"
          >
            <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate">{repository.name}</span>
          </Button>
        ))
      ) : (
        <EmptyResourceState>
          No repositories are related yet.
        </EmptyResourceState>
      )}
    </ResourceViewShell>
  );
}

function ResourceViewShell({
  action,
  children,
  description,
  testId,
  title,
}: {
  action: React.ReactNode;
  children: React.ReactNode;
  description: string;
  testId: string;
  title: string;
}) {
  return (
    <section
      className="min-h-0 flex-1 overflow-y-auto px-5 py-4"
      data-testid={testId}
    >
      <div className="mx-auto w-full max-w-4xl">
        <header className="flex min-w-0 items-center justify-between gap-4 border-b border-border/60 pb-3">
          <div className="min-w-0">
            <h2 className="truncate text-base font-semibold">{title}</h2>
            <p className="mt-0.5 text-sm text-muted-foreground">
              {description}
            </p>
          </div>
          <div className="shrink-0">{action}</div>
        </header>
        <div>{children}</div>
      </div>
    </section>
  );
}

function EmptyResourceState({ children }: { children: React.ReactNode }) {
  return <p className="py-8 text-sm text-muted-foreground">{children}</p>;
}
