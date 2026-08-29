import {
  ChevronDown,
  CircleDot,
  FileCode2,
  FolderGit2,
  GitCommitHorizontal,
  GitPullRequest,
  Hash,
  Users,
} from "lucide-react";
import * as React from "react";

import { presentContextCount } from "@/features/projects/lib/projectHomeSummary";
import type { ProjectHomeWorkspaceSheetTab } from "@/features/projects/lib/projectHomeWorkspaceSheet";
import { resolveProjectDefaultBranch } from "@/features/projects/lib/projectBranches";
import { listProjectBoundChannels } from "@/features/projects/lib/projectRelatedChannels";
import {
  useProjectActivitySummariesQuery,
  useProjectRepoSnapshotQuery,
  useRepoStateQuery,
  type Project,
} from "@/features/projects/hooks";
import { ProjectChannelIcon } from "@/features/projects/ui/ProjectChannelIcon";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { Button } from "@/shared/ui/button";
import { ProjectChannelManagement } from "./ProjectChannelManagement";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";
import { SECTION_ACTION_VISIBILITY_CLASS } from "@/features/sidebar/ui/sidebarSectionStyles";

const PROJECT_HOME_SIDEBAR_ROW_CLASS =
  "h-8 w-full justify-start gap-2 rounded-md px-2 py-1.5 text-left text-sm font-normal text-sidebar-foreground/80 transition-[background-color,color] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground disabled:pointer-events-none disabled:opacity-50";

function ContextSection({
  children,
  collapsible = false,
  headerAction,
  testId,
  title,
}: {
  children: React.ReactNode;
  collapsible?: boolean;
  headerAction?: React.ReactNode;
  testId?: string;
  title?: string;
}) {
  const [expanded, setExpanded] = React.useState(true);
  return (
    <section className="group/sidebar-section space-y-1" data-testid={testId}>
      {title || headerAction ? (
        <div className="flex h-8 min-w-0 items-center justify-between gap-2 px-2">
          {title && collapsible ? (
            <button
              aria-expanded={expanded}
              className="group/section-label flex min-w-0 items-center gap-1 text-left text-xs font-medium text-sidebar-foreground/70"
              onClick={() => setExpanded((current) => !current)}
              type="button"
            >
              <span className="truncate">{title}</span>
              <ChevronDown
                aria-hidden="true"
                className={cn(
                  "size-3 shrink-0 opacity-0 transition-[opacity,transform] group-hover/sidebar-section:opacity-100 group-focus-within/sidebar-section:opacity-100",
                  expanded ? "rotate-0" : "-rotate-90",
                )}
              />
            </button>
          ) : title ? (
            <h3 className="min-w-0 truncate text-xs font-medium text-sidebar-foreground/70">
              {title}
            </h3>
          ) : (
            <span />
          )}
          {headerAction ? (
            <span className={SECTION_ACTION_VISIBILITY_CLASS}>
              {headerAction}
            </span>
          ) : null}
        </div>
      ) : null}
      {!collapsible || expanded ? children : null}
    </section>
  );
}

function ContextRowContent({
  children,
  count,
  icon,
}: {
  children: React.ReactNode;
  count?: number;
  icon: React.ReactNode;
}) {
  return (
    <>
      <span className="flex size-4 shrink-0 items-center justify-center text-current">
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate text-left">{children}</span>
      <span className="w-8 shrink-0 text-right tabular-nums text-current opacity-60">
        {count ?? ""}
      </span>
    </>
  );
}

function ContextNavButton({
  children,
  count,
  disabled,
  icon,
  onClick,
  pressed,
  testId,
  title,
}: {
  children: React.ReactNode;
  count?: number;
  disabled?: boolean;
  icon: React.ReactNode;
  onClick?: () => void;
  pressed?: boolean;
  testId?: string;
  title?: string;
}) {
  return (
    <Button
      aria-pressed={pressed}
      className={cn(
        PROJECT_HOME_SIDEBAR_ROW_CLASS,
        pressed &&
          "bg-sidebar-active text-sidebar-active-foreground shadow-xs hover:bg-sidebar-active hover:text-sidebar-active-foreground",
      )}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      size="sm"
      title={title}
      type="button"
      variant="ghost"
    >
      <ContextRowContent count={count} icon={icon}>
        {children}
      </ContextRowContent>
    </Button>
  );
}

function ChannelContextRow({
  channel,
  onClick,
  projectHome,
  testId,
}: {
  channel: Channel;
  onClick?: () => void;
  projectHome?: boolean;
  testId: string;
}) {
  const Icon = projectHome ? ProjectChannelIcon : Hash;
  if (onClick) {
    return (
      <ContextNavButton icon={<Icon />} onClick={onClick} testId={testId}>
        {channel.name}
      </ContextNavButton>
    );
  }
  return (
    <div
      className={`${PROJECT_HOME_SIDEBAR_ROW_CLASS} pointer-events-none flex items-center`}
      data-testid={testId}
    >
      <ContextRowContent icon={<Icon />}>{channel.name}</ContextRowContent>
    </div>
  );
}

export function ProjectHomeContextPanel({
  activeWorkspaceTab,
  channel,
  channels = [],
  identityPubkey,
  onAddRepository,
  onOpenChannel,
  onOpenRepository,
  onOpenWorkspace,
  onRepositoryChange,
  project,
  projects,
}: {
  activeWorkspaceTab?: ProjectHomeWorkspaceSheetTab | null;
  channel: Channel | null;
  channels?: Channel[];
  identityPubkey?: string;
  onAddRepository?: () => void;
  onOpenChannel?: (channelId: string) => void;
  onOpenRepository: (repositoryId: string) => void;
  onOpenWorkspace: (repositoryId: string, tab?: EntityLinkTab) => void;
  onRepositoryChange: (repositoryId: string) => void;
  project: Project;
  projects: Project[];
}) {
  const firstRepository = project.repositories[0] ?? null;
  const addRepositoryTitle = firstRepository
    ? undefined
    : "Add a repository to this project";
  const openWorkspace = (tab: EntityLinkTab) => {
    if (firstRepository) {
      onOpenWorkspace(firstRepository.id, tab);
      return;
    }
    onAddRepository?.();
  };
  const peopleCount = new Set([
    project.owner,
    ...project.repositories.flatMap((repository) => repository.contributors),
  ]).size;
  const activityQuery = useProjectActivitySummariesQuery([project]);
  const activity = activityQuery.data?.[project.id];
  const repoStateQuery = useRepoStateQuery(firstRepository);
  const defaultBranch = firstRepository
    ? resolveProjectDefaultBranch(
        firstRepository.defaultBranch,
        repoStateQuery.data,
      )
    : null;
  const snapshotQuery = useProjectRepoSnapshotQuery(
    firstRepository,
    defaultBranch,
    null,
    null,
    Boolean(firstRepository),
  );
  const channelsById = new Map(
    channels.map((candidate) => [candidate.id, candidate]),
  );
  const boundChannels = listProjectBoundChannels(project).flatMap((binding) => {
    const boundChannel = channelsById.get(binding.channelId);
    if (!boundChannel) return [];
    return [{ ...binding, channel: boundChannel }];
  });
  const listedChannels =
    boundChannels.length > 0
      ? boundChannels
      : channel
        ? [
            {
              channel,
              channelId: channel.id,
              repositoryId: null,
              role: "home" as const,
            },
          ]
        : [];

  return (
    <div
      className="space-y-4 px-2 pb-8 pt-3"
      data-testid="project-home-context-panel"
    >
      <ContextSection testId="project-home-context-workspace">
        <ContextNavButton
          count={presentContextCount(activity?.issueCount)}
          disabled={!firstRepository && !onAddRepository}
          icon={<CircleDot />}
          onClick={() => openWorkspace("issues")}
          pressed={activeWorkspaceTab === "issues"}
          testId="project-home-context-tasks"
          title={addRepositoryTitle}
        >
          Tasks
        </ContextNavButton>
        <ContextNavButton
          count={presentContextCount(activity?.prCount)}
          disabled={!firstRepository && !onAddRepository}
          icon={<GitPullRequest />}
          onClick={() => openWorkspace("prs")}
          pressed={activeWorkspaceTab === "prs"}
          testId="project-home-context-reviews"
          title={addRepositoryTitle}
        >
          Reviews
        </ContextNavButton>
        <ContextNavButton
          count={presentContextCount(activity?.commitCount)}
          disabled={!firstRepository && !onAddRepository}
          icon={<GitCommitHorizontal />}
          onClick={() => openWorkspace("commits")}
          pressed={activeWorkspaceTab === "commits"}
          testId="project-home-context-commits"
          title={addRepositoryTitle}
        >
          Commits
        </ContextNavButton>
        <ContextNavButton
          count={presentContextCount(snapshotQuery.data?.files.length)}
          disabled={!firstRepository && !onAddRepository}
          icon={<FileCode2 />}
          onClick={() => openWorkspace("files")}
          pressed={activeWorkspaceTab === "files"}
          testId="project-home-context-files"
          title={addRepositoryTitle}
        >
          Files
        </ContextNavButton>
        <ContextNavButton
          count={presentContextCount(peopleCount)}
          disabled={!firstRepository}
          icon={<Users />}
          onClick={() =>
            firstRepository &&
            onOpenWorkspace(firstRepository.id, "contributors")
          }
          pressed={activeWorkspaceTab === "contributors"}
          testId="project-home-context-people"
          title={addRepositoryTitle}
        >
          People
        </ContextNavButton>
      </ContextSection>
      <ContextSection
        collapsible
        headerAction={
          <ProjectChannelManagement
            identityPubkey={identityPubkey}
            project={project}
          />
        }
        testId="project-home-context-channel"
        title="Channels"
      >
        {listedChannels.length > 0 ? (
          listedChannels.map((binding) => {
            const isHome = binding.role === "home";
            return (
              <ChannelContextRow
                channel={binding.channel}
                key={`${binding.role}:${binding.channel.id}`}
                onClick={
                  isHome || !onOpenChannel
                    ? undefined
                    : () => onOpenChannel(binding.channel.id)
                }
                projectHome={isHome}
                testId={
                  isHome
                    ? "project-home-context-home-channel"
                    : `project-home-context-channel-${binding.channel.name}`
                }
              />
            );
          })
        ) : (
          <p
            className={`${PROJECT_HOME_SIDEBAR_ROW_CLASS} pointer-events-none flex items-center`}
          >
            <ContextRowContent icon={<Hash />}>Unavailable</ContextRowContent>
          </p>
        )}
      </ContextSection>
      <ContextSection
        collapsible
        headerAction={
          <ProjectRepositoryManagement
            compact
            identityPubkey={identityPubkey}
            onChange={onRepositoryChange}
            project={project}
            projects={projects}
          />
        }
        testId="project-home-context-codebase"
        title="Codebase"
      >
        {project.repositories.length > 0 ? (
          project.repositories.map((repository) => (
            <ContextNavButton
              icon={<FolderGit2 />}
              key={repository.id}
              onClick={() => onOpenRepository(repository.id)}
              testId={`project-home-context-repo-${repository.dtag}`}
            >
              {repository.name}
            </ContextNavButton>
          ))
        ) : (
          <p className="px-2 py-1 text-sm text-sidebar-foreground/60">
            None yet
          </p>
        )}
      </ContextSection>
    </div>
  );
}
