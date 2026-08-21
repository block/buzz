import * as React from "react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  Project,
  ProjectActivitySummary,
  Repository,
} from "@/features/projects/hooks";
import {
  hasLocalCheckout,
  hasLocalRepositoryCheckout,
} from "@/features/projects/lib/projectLocalRepos";
import type { ProjectRepoUnavailableReason } from "@/features/projects/lib/projectRepoAvailability";
import {
  projectShareLink,
  repositoryShareLink,
} from "@/features/projects/lib/projectShareLinks";
import {
  isProjectOwnedByCurrentUser,
  projectPeople,
  type ProjectsFilter,
  type ProjectsViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import {
  selectionItemFromProject,
  selectionItemFromRepository,
} from "@/features/projects/lib/projectSelection";
import {
  EmptyFilteredState,
  ProjectGridCard,
  ProjectListRow,
} from "@/features/projects/ui/ProjectCards";
import {
  RepositoryGridCard,
  RepositoryListRow,
} from "@/features/projects/ui/RepositoryCards";
import { cn } from "@/shared/lib/cn";

// Stable fallback so a cache miss cannot hand a memoized card a fresh array.
const EMPTY_PEOPLE: string[] = [];

export function ProjectsOverviewProjectItems({
  currentPubkey,
  deleteDisabled,
  filter,
  localRepoNames,
  onDelete,
  onOpen,
  onOpenTerminal,
  profiles,
  repositoryUnavailableReasonFor,
  summaries,
  viewMode,
  visibleProjects,
}: {
  currentPubkey: string | undefined;
  deleteDisabled: boolean;
  filter: ProjectsFilter;
  localRepoNames: Set<string>;
  onDelete: (project: Project) => void;
  onOpen: (project: Project) => void;
  onOpenTerminal: (project: Project) => void;
  profiles?: UserProfileLookup;
  repositoryUnavailableReasonFor: (
    project: Project,
  ) => ProjectRepoUnavailableReason | undefined;
  summaries?: Record<string, ProjectActivitySummary>;
  viewMode: ProjectsViewMode;
  visibleProjects: Project[];
}) {
  // One selection array shared by every row (was rebuilt per row per render —
  // O(n²) object churn that also defeated row memoization).
  const selectionRangeItems = React.useMemo(
    () =>
      visibleProjects.map((item) =>
        selectionItemFromProject({
          channelId: item.projectChannelId,
          id: item.id,
          owner: item.owner,
          shareLink: projectShareLink(item),
          title: item.name,
        }),
      ),
    [visibleProjects],
  );
  // Identity-stable people arrays: an inline projectPeople() call would hand
  // every memoized card a fresh array each render, defeating React.memo.
  const peopleByProject = React.useMemo(
    () =>
      new Map(
        visibleProjects.map((project) => [
          project.id,
          projectPeople(project, summaries?.[project.id]),
        ]),
      ),
    [summaries, visibleProjects],
  );
  if (visibleProjects.length === 0) {
    return <EmptyFilteredState />;
  }
  if (viewMode === "grid") {
    return (
      <div
        className={cn(
          "grid gap-3 md:grid-cols-2",
          filter !== "all" && "xl:grid-cols-3",
        )}
      >
        {visibleProjects.map((project) => {
          const summary = summaries?.[project.id];
          return (
            <div
              className={
                "[contain-intrinsic-size:auto_11rem] [content-visibility:auto]"
              }
              key={project.id}
            >
              <ProjectGridCard
                canDelete={isProjectOwnedByCurrentUser(project, currentPubkey)}
                deleteDisabled={deleteDisabled}
                hasLocal={hasLocalCheckout(project, localRepoNames)}
                onDelete={onDelete}
                onOpen={onOpen}
                onOpenTerminal={onOpenTerminal}
                people={peopleByProject.get(project.id) ?? EMPTY_PEOPLE}
                profiles={profiles}
                project={project}
                repositoryUnavailableReason={repositoryUnavailableReasonFor(
                  project,
                )}
                summary={summary}
              />
            </div>
          );
        })}
      </div>
    );
  }
  return (
    <div data-testid="projects-list-container">
      {visibleProjects.map((project) => {
        const summary = summaries?.[project.id];
        return (
          <div
            className={
              "[contain-intrinsic-size:auto_3.5rem] [content-visibility:auto]"
            }
            key={project.id}
          >
            <ProjectListRow
              canDelete={isProjectOwnedByCurrentUser(project, currentPubkey)}
              deleteDisabled={deleteDisabled}
              hasLocal={hasLocalCheckout(project, localRepoNames)}
              onDelete={onDelete}
              onOpen={onOpen}
              onOpenTerminal={onOpenTerminal}
              people={peopleByProject.get(project.id) ?? EMPTY_PEOPLE}
              profiles={profiles}
              project={project}
              repositoryUnavailableReason={repositoryUnavailableReasonFor(
                project,
              )}
              selectionRangeItems={selectionRangeItems}
              summary={summary}
            />
          </div>
        );
      })}
    </div>
  );
}

export function ProjectsOverviewRepositoryItems({
  localRepoNames,
  onOpen,
  onOpenTerminal,
  profiles,
  summaries,
  viewMode,
  visibleRepositories,
}: {
  localRepoNames: Set<string>;
  onOpen: (project: Project, repository: Repository) => void;
  onOpenTerminal: (repository: Repository) => void;
  profiles?: UserProfileLookup;
  summaries?: Record<string, ProjectActivitySummary>;
  viewMode: ProjectsViewMode;
  visibleRepositories: Array<{ project: Project; repository: Repository }>;
}) {
  // Shared, identity-stable selection array (see ProjectsOverviewProjectItems).
  const selectionRangeItems = React.useMemo(
    () =>
      visibleRepositories.map((row) =>
        selectionItemFromRepository({
          channelId: row.repository.channelId ?? row.project.projectChannelId,
          id: row.repository.id,
          owner: row.repository.owner,
          shareLink: repositoryShareLink(row.repository),
          title: row.repository.name,
        }),
      ),
    [visibleRepositories],
  );
  if (visibleRepositories.length === 0) {
    return <EmptyFilteredState />;
  }
  if (viewMode === "grid") {
    return (
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {visibleRepositories.map(({ project, repository }) => (
          <div
            className={
              "[contain-intrinsic-size:auto_11rem] [content-visibility:auto]"
            }
            key={repository.repoAddress}
          >
            <RepositoryGridCard
              hasLocal={hasLocalRepositoryCheckout(repository, localRepoNames)}
              onOpen={onOpen}
              onOpenTerminal={onOpenTerminal}
              profiles={profiles}
              project={project}
              repository={repository}
              summary={summaries?.[repository.repoAddress]}
            />
          </div>
        ))}
      </div>
    );
  }
  return (
    <div data-testid="projects-list-container">
      {visibleRepositories.map(({ project, repository }) => (
        <div
          className={
            "[contain-intrinsic-size:auto_3.5rem] [content-visibility:auto]"
          }
          key={repository.repoAddress}
        >
          <RepositoryListRow
            hasLocal={hasLocalRepositoryCheckout(repository, localRepoNames)}
            onOpen={onOpen}
            onOpenTerminal={onOpenTerminal}
            profiles={profiles}
            project={project}
            repository={repository}
            selectionRangeItems={selectionRangeItems}
            summary={summaries?.[repository.repoAddress]}
          />
        </div>
      ))}
    </div>
  );
}
