import { FolderGit2, GitBranch, Globe, SquareTerminal } from "lucide-react";

import type {
  Project,
  ProjectActivitySummary,
  Repository,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import {
  projectRepoHostForRepository,
  repositoryDisplayPath,
} from "@/features/projects/lib/projectRepoHost";
import {
  formatExactTimestamp,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  PROJECT_LIST_ROW_CLASS,
  PROJECT_LIST_ROW_DATE_CLASS,
  PROJECT_LIST_ROW_PREVIEW_CLASS,
  PROJECT_LIST_ROW_TITLE_CLASS,
  PROJECT_LIST_ROW_TRAILING_CLASS,
} from "./projectListRowStyles";
import {
  ProjectActivityBar,
  ProjectPeopleStack,
  ProjectStatsRow,
} from "./ProjectCards";
import { GitHubMark } from "./GitHubMark";
import { ProjectListRowMenu } from "./ProjectListRowMenu";
import { projectTerminalLabel } from "./useOpenProjectTerminal";

export type RepositoryListItem = {
  project: Project;
  repository: Repository;
};

type RepositoryItemProps = RepositoryListItem & {
  hasLocal: boolean;
  onOpen: (project: Project, repository: Repository) => void;
  onOpenTerminal: (repository: Repository) => void;
  profiles?: UserProfileLookup;
  summary?: ProjectActivitySummary;
};

function RepositoryHostIcon({ repository }: { repository: Repository }) {
  const host = projectRepoHostForRepository(repository, useRelayOrigin());
  const label =
    host.kind === "buzz"
      ? "Buzz-hosted repository"
      : host.kind === "external"
        ? `Git data hosted on ${host.host}`
        : "Repository host";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          aria-label={label}
          className="pointer-events-auto flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-muted-foreground"
          data-testid="repository-host-icon"
          role="img"
        >
          {host.kind === "buzz" ? (
            <BuzzMark className="h-4.5 w-5" />
          ) : host.kind === "external" && host.host === "github.com" ? (
            <GitHubMark className="h-4.5 w-4.5" />
          ) : host.kind === "external" ? (
            <Globe className="h-4.5 w-4.5" />
          ) : (
            <FolderGit2 className="h-4.5 w-4.5" />
          )}
        </span>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function RepositoryOpenButton({
  onOpen,
  project,
  repository,
}: Pick<RepositoryItemProps, "onOpen" | "project" | "repository">) {
  return (
    <button
      className="absolute inset-0 z-0 cursor-pointer"
      onClick={() => onOpen(project, repository)}
      type="button"
    >
      <span className="sr-only">View {repository.name}</span>
    </button>
  );
}

function RepositoryIdentity({
  inlineBranch = false,
  profiles,
  project,
  repository,
}: Pick<RepositoryItemProps, "profiles" | "project" | "repository"> & {
  inlineBranch?: boolean;
}) {
  // Where the git data lives beats repeating the (often identical) project
  // name — "github.com/block/buzz" for external repos, "owner/repo" for
  // Buzz-hosted ones.
  const displayPath = repositoryDisplayPath(
    repository,
    useRelayOrigin(),
    resolveUserLabel({ pubkey: repository.owner, profiles }),
  );
  return (
    <>
      <RepositoryHostIcon repository={repository} />
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <span className={PROJECT_LIST_ROW_TITLE_CLASS}>
            {repository.name}
          </span>
        </div>
        <p className={PROJECT_LIST_ROW_PREVIEW_CLASS}>
          {displayPath ?? project.name}
          {inlineBranch ? ` · ${repository.defaultBranch}` : ""}
        </p>
      </div>
    </>
  );
}

function RepositoryUpdatedLabel({
  repository,
  summary,
}: Pick<RepositoryItemProps, "repository" | "summary">) {
  const updatedAt = summary?.updatedAt || repository.createdAt;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="whitespace-nowrap text-xs leading-4 text-muted-foreground/70">
          {relativeTime(updatedAt)}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {summary?.latestCommit
          ? `${summary.latestCommit.title || summary.latestCommit.commit.slice(0, 7)} · ${formatExactTimestamp(summary.latestCommit.createdAt)}`
          : `Created ${formatExactTimestamp(repository.createdAt)}`}
      </TooltipContent>
    </Tooltip>
  );
}

function repositoryPeople(
  repository: Repository,
  summary: ProjectActivitySummary | undefined,
) {
  return [
    ...new Set(
      [
        repository.owner,
        ...repository.contributors,
        ...(summary?.participantPubkeys ?? []),
      ].map(normalizePubkey),
    ),
  ];
}

function RepositoryActionsMenu({
  hasLocal,
  onOpenTerminal,
  repository,
}: Pick<RepositoryItemProps, "hasLocal" | "onOpenTerminal" | "repository">) {
  return (
    <ProjectListRowMenu label={`More options for ${repository.name}`}>
      <DropdownMenuItem
        onSelect={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onOpenTerminal(repository);
        }}
      >
        <SquareTerminal className="h-4 w-4" />
        {projectTerminalLabel(hasLocal)}
      </DropdownMenuItem>
    </ProjectListRowMenu>
  );
}

export function RepositoryGridCard(props: RepositoryItemProps) {
  const {
    hasLocal,
    onOpen,
    onOpenTerminal,
    profiles,
    project,
    repository,
    summary,
  } = props;
  return (
    <Card
      className="group relative flex min-h-40 flex-col overflow-hidden border-border/60 bg-transparent shadow-none transition-colors duration-150 hover:bg-muted/20"
      data-testid={`repository-card-${repository.dtag}`}
    >
      <RepositoryOpenButton
        onOpen={onOpen}
        project={project}
        repository={repository}
      />
      <div className="pointer-events-none relative z-10 flex min-h-0 flex-1 flex-col p-4">
        <div className="flex min-w-0 items-start gap-2.5">
          <RepositoryIdentity
            inlineBranch
            profiles={profiles}
            project={project}
            repository={repository}
          />
          <div className="pointer-events-auto ml-auto">
            <RepositoryActionsMenu
              hasLocal={hasLocal}
              onOpenTerminal={onOpenTerminal}
              repository={repository}
            />
          </div>
        </div>
        <p className="line-clamp-2 min-h-10 py-3 text-sm text-muted-foreground">
          {repository.description || "A repository in this project."}
        </p>
        <div className="pointer-events-auto mt-auto flex items-center justify-between gap-3">
          <ProjectPeopleStack
            profiles={profiles}
            pubkeys={repositoryPeople(repository, summary)}
            workOwnerPubkey={repository.owner}
          />
          <RepositoryUpdatedLabel repository={repository} summary={summary} />
        </div>
        <div className="mt-2">
          <ProjectStatsRow summary={summary} />
          <div className="mt-2">
            <ProjectActivityBar summary={summary} />
          </div>
        </div>
      </div>
    </Card>
  );
}

export function RepositoryListRow(props: RepositoryItemProps) {
  const {
    hasLocal,
    onOpen,
    onOpenTerminal,
    profiles,
    project,
    repository,
    summary,
  } = props;
  return (
    <div
      className={cn(PROJECT_LIST_ROW_CLASS, "py-3")}
      data-testid={`repository-row-${repository.dtag}`}
    >
      <RepositoryOpenButton
        onOpen={onOpen}
        project={project}
        repository={repository}
      />
      <div className="pointer-events-none relative z-10 flex min-w-0 flex-1 items-center gap-2.5">
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <RepositoryIdentity
            profiles={profiles}
            project={project}
            repository={repository}
          />
        </div>
        <div
          className={cn(PROJECT_LIST_ROW_TRAILING_CLASS, "pointer-events-auto")}
        >
          <div
            className="hidden w-24 shrink-0 items-center gap-1.5 text-xs text-muted-foreground md:flex"
            data-testid="repositories-row-branch"
          >
            <GitBranch className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{repository.defaultBranch}</span>
          </div>
          <div
            className="hidden items-center gap-3 xl:flex"
            data-testid="repositories-row-summary"
          >
            <ProjectStatsRow fixedColumns summary={summary} />
            <div className="w-20 shrink-0">
              <ProjectActivityBar summary={summary} />
            </div>
          </div>
          <div
            className="hidden w-24 shrink-0 justify-end lg:flex"
            data-testid="repositories-row-people"
          >
            <ProjectPeopleStack
              profiles={profiles}
              pubkeys={repositoryPeople(repository, summary)}
              workOwnerPubkey={repository.owner}
            />
          </div>
          <div
            className={PROJECT_LIST_ROW_DATE_CLASS}
            data-testid="repositories-row-date"
          >
            <RepositoryUpdatedLabel repository={repository} summary={summary} />
          </div>
          <RepositoryActionsMenu
            hasLocal={hasLocal}
            onOpenTerminal={onOpenTerminal}
            repository={repository}
          />
        </div>
      </div>
    </div>
  );
}
