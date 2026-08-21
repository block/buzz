import { FolderGit2, Globe, SquareTerminal } from "lucide-react";

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
import { repositoryShareLink } from "@/features/projects/lib/projectShareLinks";
import {
  selectionItemFromRepository,
  type ProjectSelectionItem,
} from "@/features/projects/lib/projectSelection";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  PROJECT_LIST_ROW_PREVIEW_CLASS,
  PROJECT_LIST_ROW_TITLE_CLASS,
} from "./projectListRowStyles";
import { ProjectEntityListRow } from "./ProjectEntityListRow";
import {
  ProjectActivityBar,
  ProjectPeopleStack,
  ProjectStatsRow,
} from "./ProjectCards";
import { CopyShareLinkMenuItem } from "./CopyShareLinkMenuItem";
import { GitHubMark } from "./GitHubMark";
import { ProjectListRowMenu } from "./ProjectListRowMenu";
import { PROJECT_GRID_CARD_BODY_CLASS } from "./projectGridCardStyles";
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
  selectionRangeItems?: ProjectSelectionItem[];
  summary?: ProjectActivitySummary;
};

function RepositoryHostIcon({
  compact = false,
  repository,
}: {
  compact?: boolean;
  repository: Repository;
}) {
  const host = projectRepoHostForRepository(repository, useRelayOrigin());
  const label =
    host.kind === "buzz"
      ? "Buzz-hosted repository"
      : host.kind === "external"
        ? `Git data hosted on ${host.host}`
        : "Repository host";
  const mark =
    host.kind === "buzz" ? (
      <BuzzMark className={compact ? "h-3.5 w-4" : "h-4.5 w-5"} />
    ) : host.kind === "external" && host.host === "github.com" ? (
      <GitHubMark className={compact ? "h-3.5 w-3.5" : "h-4.5 w-4.5"} />
    ) : host.kind === "external" ? (
      <Globe className={compact ? "h-3.5 w-3.5" : "h-4.5 w-4.5"} />
    ) : (
      <FolderGit2 className={compact ? "h-3.5 w-3.5" : "h-4.5 w-4.5"} />
    );

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          aria-label={label}
          className={
            compact
              ? "pointer-events-auto flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground/70"
              : "pointer-events-auto flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-muted-foreground"
          }
          data-testid="repository-host-icon"
          role="img"
        >
          {mark}
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
      <CopyShareLinkMenuItem
        link={repositoryShareLink(repository)}
        testId={`repository-copy-link-${repository.dtag}`}
      />
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
        <p
          className={cn(
            PROJECT_GRID_CARD_BODY_CLASS,
            "my-2 text-muted-foreground",
          )}
          data-testid="projects-grid-card-body"
        >
          {repository.description || "A repository in this project."}
        </p>
        <div className="pointer-events-auto mt-auto">
          <ProjectPeopleStack
            profiles={profiles}
            pubkeys={repositoryPeople(repository, summary)}
            workOwnerPubkey={repository.owner}
          />
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
    selectionRangeItems,
    summary,
  } = props;
  const updatedAt = summary?.updatedAt || repository.createdAt;
  const selectionItem = selectionItemFromRepository({
    channelId: repository.channelId ?? project.projectChannelId,
    id: repository.id,
    owner: repository.owner,
    shareLink: repositoryShareLink(repository),
    title: repository.name,
  });
  return (
    <ProjectEntityListRow
      beforeDate={
        <div className="w-44" data-testid="repositories-row-activity-bar">
          <ProjectActivityBar summary={summary} />
        </div>
      }
      dateSeconds={updatedAt}
      dateTestId="repositories-row-date"
      icon={<RepositoryHostIcon compact repository={repository} />}
      onClick={() => onOpen(project, repository)}
      people={repositoryPeople(repository, summary)}
      peopleTestId="repositories-row-people"
      profiles={profiles}
      selection={
        selectionRangeItems
          ? { item: selectionItem, rangeItems: selectionRangeItems }
          : undefined
      }
      testId={`repository-row-${repository.dtag}`}
      title={repository.name}
      titleAttr={repository.name}
      titleSecondary={repository.description || undefined}
      titleSecondaryTestId="repositories-row-description"
      trailing={
        <RepositoryActionsMenu
          hasLocal={hasLocal}
          onOpenTerminal={onOpenTerminal}
          repository={repository}
        />
      }
    />
  );
}
