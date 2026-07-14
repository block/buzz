import {
  CircleDot,
  FolderGit2,
  GitCommit,
  GitPullRequest,
  MoreHorizontal,
  TerminalSquare,
  Trash2,
} from "lucide-react";
import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type {
  Project,
  ProjectActivitySummary,
} from "@/features/projects/hooks";
import {
  formatExactTimestamp,
  getProjectUpdatedAt,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import { projectTerminalLabel } from "@/features/projects/ui/useOpenProjectTerminal";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function ProjectUpdatedLabel({
  profiles,
  project,
  summary,
}: {
  profiles?: UserProfileLookup;
  project: Project;
  summary: ProjectActivitySummary | undefined;
}) {
  const updatedAt = getProjectUpdatedAt(project, summary);
  const latestCommit = summary?.latestCommit;
  const authorLabel = latestCommit?.author
    ? resolveUserLabel({ profiles, pubkey: latestCommit.author })
    : null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="whitespace-nowrap text-xs text-muted-foreground">
          {relativeTime(updatedAt)}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-96 break-words">
        {latestCommit
          ? `${latestCommit.title || latestCommit.commit.slice(0, 7)}${
              authorLabel ? ` · ${authorLabel}` : ""
            } · ${formatExactTimestamp(latestCommit.createdAt)}`
          : `Created ${formatExactTimestamp(project.createdAt)}`}
      </TooltipContent>
    </Tooltip>
  );
}

function ProjectPeopleStack({
  pubkeys,
  profiles,
  workOwnerPubkey,
}: {
  pubkeys: string[];
  profiles?: UserProfileLookup;
  workOwnerPubkey: string;
}) {
  const visible = pubkeys.slice(0, 5);
  const remaining = pubkeys.length - visible.length;

  if (visible.length === 0) {
    return null;
  }

  return (
    <div className="flex items-center justify-end -space-x-1.5">
      {visible.map((pubkey, index) => {
        const profile = profiles?.[normalizePubkey(pubkey)];
        const label = resolveUserLabel({ pubkey, profiles });
        return (
          <Tooltip key={pubkey}>
            <TooltipTrigger asChild>
              {/* First avatar sits on the top layer, cascading down rightward. */}
              <span
                className="relative inline-flex"
                style={{ zIndex: visible.length - index }}
              >
                <UserAvatar
                  accent={
                    normalizePubkey(pubkey) === normalizePubkey(workOwnerPubkey)
                  }
                  avatarUrl={profile?.avatarUrl ?? null}
                  className="ring-2 ring-card"
                  displayName={label}
                  size="xs"
                />
              </span>
            </TooltipTrigger>
            <TooltipContent>{label}</TooltipContent>
          </Tooltip>
        );
      })}
      {remaining > 0 ? (
        <span className="relative z-0 flex h-5 min-w-5 items-center justify-center rounded-sm bg-muted px-1 text-3xs font-semibold text-muted-foreground ring-2 ring-card">
          +{remaining}
        </span>
      ) : null}
    </div>
  );
}

const PROJECT_STAT_ITEMS = [
  {
    key: "commitCount",
    icon: GitCommit,
    iconClass: "text-primary/60",
    barClass: "bg-primary/60",
    columnClass: "w-24",
    label: (count: number) => (count === 1 ? "commit" : "commits"),
  },
  {
    key: "prCount",
    icon: GitPullRequest,
    iconClass: "text-primary",
    barClass: "bg-primary",
    columnClass: "w-16",
    label: (count: number) => (count === 1 ? "PR" : "PRs"),
  },
  {
    key: "issueCount",
    icon: CircleDot,
    iconClass: "text-orange-500",
    barClass: "bg-orange-500",
    columnClass: "w-20",
    label: (count: number) => (count === 1 ? "issue" : "issues"),
  },
] as const;

function ProjectStatsRow({
  summary,
  fixedColumns = false,
}: {
  summary: ProjectActivitySummary | undefined;
  /** Give each stat a fixed width so stats align vertically across list rows. */
  fixedColumns?: boolean;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-x-3 gap-y-1 text-xs text-muted-foreground",
        !fixedColumns && "flex-wrap",
      )}
    >
      {PROJECT_STAT_ITEMS.map(
        ({ key, icon: Icon, iconClass, label, columnClass }) => {
          const count = summary?.[key] ?? 0;
          return (
            <span
              className={cn(
                "flex items-center gap-1",
                fixedColumns && cn("shrink-0", columnClass),
              )}
              key={key}
            >
              <Icon className={cn("h-3.5 w-3.5 shrink-0", iconClass)} />
              <span className="font-medium text-foreground">{count}</span>
              {label(count)}
            </span>
          );
        },
      )}
    </div>
  );
}

// Segmented commits/PRs/issues distribution — the card's "progress bar".
// Hovering thickens the bar and reveals a tooltip with the exact breakdown.
function ProjectActivityBar({
  summary,
}: {
  summary: ProjectActivitySummary | undefined;
}) {
  const items = PROJECT_STAT_ITEMS.map(({ key, barClass, label }) => {
    const count = summary?.[key] ?? 0;
    return { barClass, count, text: label(count) };
  });
  const total = items.reduce((sum, item) => sum + item.count, 0);

  return (
    // z-10 lifts the bar above the card's full-surface open button so it
    // can receive hover events. Fixed h-2 wrapper keeps layout stable
    // while the inner bar grows on hover.
    <div className="group/activity-bar relative z-10 flex h-2 w-full items-center">
      <div className="flex h-1.5 w-full gap-px overflow-hidden rounded-full bg-muted/60 transition-all duration-150 group-hover/activity-bar:h-2">
        {total > 0
          ? items
              .filter((item) => item.count > 0)
              .map((item) => (
                <Tooltip key={item.barClass}>
                  <TooltipTrigger asChild>
                    <div
                      className={cn("h-full", item.barClass)}
                      style={{ width: `${(item.count / total) * 100}%` }}
                    />
                  </TooltipTrigger>
                  <TooltipContent>
                    <span className="flex items-center gap-1.5">
                      <span
                        className={cn("h-2 w-2 rounded-full", item.barClass)}
                      />
                      {item.count} {item.text}
                    </span>
                  </TooltipContent>
                </Tooltip>
              ))
          : null}
      </div>
    </div>
  );
}

function StatusPill({ status }: { status: string }) {
  if (status === "active") {
    return null;
  }

  return (
    <span className="shrink-0 rounded-full border border-border/60 bg-muted/40 px-2 pb-[3px] pt-[5px] text-2xs font-semibold uppercase leading-none tracking-[0.18em] text-muted-foreground">
      {status}
    </span>
  );
}

export function EmptyState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
      <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
      <div className="space-y-1">
        <p className="text-sm font-medium text-foreground">No projects yet</p>
        <p className="text-sm text-muted-foreground">
          Projects published to this relay will appear here.
        </p>
      </div>
    </div>
  );
}

export function EmptyFilteredState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 border border-dashed border-border/60 px-4 py-12 text-center">
      <FolderGit2 className="h-9 w-9 text-muted-foreground/40" />
      <div className="space-y-1">
        <p className="text-sm font-medium text-foreground">
          No matching projects
        </p>
        <p className="text-sm text-muted-foreground">
          Try another owner filter or sort mode.
        </p>
      </div>
    </div>
  );
}

function ProjectCardButton({
  project,
  onOpen,
}: {
  project: Project;
  onOpen: (project: Project) => void;
}) {
  return (
    <button
      className="absolute inset-0"
      onClick={() => onOpen(project)}
      type="button"
    >
      <span className="sr-only">View {project.name}</span>
    </button>
  );
}

function ProjectActionsMenu({
  project,
  hasLocal,
  canDelete,
  disabled,
  onDelete,
  onOpenTerminal,
}: {
  project: Project;
  hasLocal: boolean;
  canDelete: boolean;
  disabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpenTerminal: (project: Project) => Promise<void> | void;
}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);

  return (
    <AlertDialog onOpenChange={setConfirmOpen} open={confirmOpen}>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={`More options for ${project.name}`}
            className="relative z-20 h-7 w-7 text-muted-foreground hover:text-foreground"
            onClick={(event) => event.stopPropagation()}
            size="icon"
            type="button"
            variant="ghost"
          >
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-48">
          <DropdownMenuItem
            onSelect={(event) => {
              event.preventDefault();
              event.stopPropagation();
              void onOpenTerminal(project);
            }}
          >
            <TerminalSquare className="h-4 w-4" />
            {projectTerminalLabel(hasLocal)}
          </DropdownMenuItem>
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={!canDelete || disabled}
            onSelect={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (canDelete && !disabled) {
                setConfirmOpen(true);
              }
            }}
          >
            <Trash2 className="h-4 w-4" />
            Delete project
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <AlertDialogContent
        data-testid={`project-delete-confirm-${project.dtag}`}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>Delete project?</AlertDialogTitle>
          <AlertDialogDescription>
            Delete {project.name} from Projects for everyone. This can only be
            done for projects you own and cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={disabled} type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid={`project-delete-confirm-button-${project.dtag}`}
              disabled={disabled}
              onClick={(event) => {
                event.preventDefault();
                void Promise.resolve(onDelete(project)).finally(() =>
                  setConfirmOpen(false),
                );
              }}
              type="button"
              variant="destructive"
            >
              {disabled ? "Deleting..." : "Delete project"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

type ProjectItemProps = {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  hasLocal: boolean;
  canDelete: boolean;
  deleteDisabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpen: (project: Project) => void;
  onOpenTerminal: (project: Project) => Promise<void> | void;
};

export function ProjectGridCard({
  project,
  people,
  profiles,
  summary,
  hasLocal,
  canDelete,
  deleteDisabled,
  onDelete,
  onOpen,
  onOpenTerminal,
}: ProjectItemProps) {
  return (
    <Card
      className="group relative flex min-h-44 flex-col overflow-hidden border-border/60 bg-card shadow-none transition-colors duration-150 hover:bg-muted/20"
      data-testid={`project-card-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex min-w-0 items-center justify-between gap-3 px-4 pt-3">
          <div className="flex min-w-0 items-center gap-2">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40">
              <FolderGit2 className="h-4.5 w-4.5 text-muted-foreground" />
            </span>
            <span className="min-w-0 truncate text-sm font-semibold text-foreground">
              {project.name}
            </span>
            <StatusPill status={project.status} />
          </div>
          <div className="relative z-10 flex shrink-0 items-center gap-1">
            <ProjectUpdatedLabel
              profiles={profiles}
              project={project}
              summary={summary}
            />
            <ProjectActionsMenu
              canDelete={canDelete}
              disabled={deleteDisabled}
              hasLocal={hasLocal}
              onDelete={onDelete}
              onOpenTerminal={onOpenTerminal}
              project={project}
            />
          </div>
        </div>

        <p className="line-clamp-2 min-h-10 px-4 py-2 text-sm text-muted-foreground">
          {project.description || "A shared space for internal git work."}
        </p>

        <div className="relative z-10 flex items-center px-4 pb-1">
          <ProjectPeopleStack
            profiles={profiles}
            pubkeys={people}
            workOwnerPubkey={project.owner}
          />
        </div>

        <div className="mt-auto">
          <div className="flex min-w-0 items-center px-4 pb-2 pt-1">
            <ProjectStatsRow summary={summary} />
          </div>
          <div className="px-4 pb-3">
            <ProjectActivityBar summary={summary} />
          </div>
        </div>
      </div>
    </Card>
  );
}

export function ProjectListRow({
  project,
  people,
  profiles,
  summary,
  hasLocal,
  canDelete,
  deleteDisabled,
  onDelete,
  onOpen,
  onOpenTerminal,
}: ProjectItemProps) {
  return (
    <div
      className="group relative px-4 py-2.5 transition-colors duration-150 hover:bg-muted/20"
      data-testid={`project-row-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40">
            <FolderGit2 className="h-4.5 w-4.5 text-muted-foreground" />
          </span>
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-semibold text-foreground">
                {project.name}
              </span>
              <StatusPill status={project.status} />
            </div>
            <p className="line-clamp-1 text-sm text-muted-foreground">
              {project.description || "A shared space for internal git work."}
            </p>
          </div>
        </div>

        <div className="relative z-10 flex shrink-0 items-center gap-3">
          <div className="hidden items-center gap-3 md:flex">
            <ProjectStatsRow fixedColumns summary={summary} />
            <div className="w-20 shrink-0">
              <ProjectActivityBar summary={summary} />
            </div>
          </div>
          <div className="flex w-24 shrink-0 justify-end">
            <ProjectPeopleStack
              profiles={profiles}
              pubkeys={people}
              workOwnerPubkey={project.owner}
            />
          </div>
          <div className="hidden w-24 shrink-0 text-right sm:block">
            <ProjectUpdatedLabel
              profiles={profiles}
              project={project}
              summary={summary}
            />
          </div>
          <ProjectActionsMenu
            canDelete={canDelete}
            disabled={deleteDisabled}
            hasLocal={hasLocal}
            onDelete={onDelete}
            onOpenTerminal={onOpenTerminal}
            project={project}
          />
        </div>
      </div>
    </div>
  );
}

/** Compact, borderless repository row for the overview side rail. */
export function ProjectRailRow({
  project,
  summary,
  onOpen,
}: Pick<ProjectItemProps, "project" | "summary" | "onOpen">) {
  return (
    <div
      className="group relative rounded-lg py-1.5 transition-colors duration-150 hover:bg-muted/30"
      data-testid={`project-rail-row-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="flex min-w-0 items-start gap-2">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted/50">
          <FolderGit2 className="h-3.5 w-3.5 text-muted-foreground" />
        </span>
        <div className="min-w-0 flex-1">
          <span className="block min-w-0 truncate text-xs font-semibold text-foreground">
            {project.name}
          </span>
          {/* Cap the bar so its right edge always lands at least one avatar
              slot (24px + 6px gap) short of the section edge, keeping it
              within the width of the People avatar row above. */}
          <div className="relative z-10 mt-1.5 w-full max-w-[calc(100%-1.875rem)]">
            <ProjectActivityBar summary={summary} />
          </div>
        </div>
      </div>
    </div>
  );
}
