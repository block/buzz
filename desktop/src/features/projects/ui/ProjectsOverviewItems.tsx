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
  isProjectOwnedByCurrentUser,
  projectPeople,
  type ProjectsFilter,
  type ProjectsViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
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
            <ProjectGridCard
              canDelete={isProjectOwnedByCurrentUser(project, currentPubkey)}
              deleteDisabled={deleteDisabled}
              hasLocal={hasLocalCheckout(project, localRepoNames)}
              key={project.id}
              onDelete={onDelete}
              onOpen={onOpen}
              onOpenTerminal={onOpenTerminal}
              people={projectPeople(project, summary)}
              profiles={profiles}
              project={project}
              repositoryUnavailableReason={repositoryUnavailableReasonFor(
                project,
              )}
              summary={summary}
            />
          );
        })}
      </div>
    );
  }
  return (
    <div
      className="divide-y divide-border/60"
      data-testid="projects-list-container"
    >
      {visibleProjects.map((project) => {
        const summary = summaries?.[project.id];
        return (
          <ProjectListRow
            canDelete={isProjectOwnedByCurrentUser(project, currentPubkey)}
            deleteDisabled={deleteDisabled}
            hasLocal={hasLocalCheckout(project, localRepoNames)}
            key={project.id}
            onDelete={onDelete}
            onOpen={onOpen}
            onOpenTerminal={onOpenTerminal}
            people={projectPeople(project, summary)}
            profiles={profiles}
            project={project}
            repositoryUnavailableReason={repositoryUnavailableReasonFor(
              project,
            )}
            summary={summary}
          />
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
  if (visibleRepositories.length === 0) {
    return <EmptyFilteredState />;
  }
  if (viewMode === "grid") {
    return (
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {visibleRepositories.map(({ project, repository }) => (
          <RepositoryGridCard
            hasLocal={hasLocalRepositoryCheckout(repository, localRepoNames)}
            key={repository.repoAddress}
            onOpen={onOpen}
            onOpenTerminal={onOpenTerminal}
            profiles={profiles}
            project={project}
            repository={repository}
            summary={summaries?.[repository.repoAddress]}
          />
        ))}
      </div>
    );
  }
  return (
    <div
      className="divide-y divide-border/60"
      data-testid="projects-list-container"
    >
      {visibleRepositories.map(({ project, repository }) => (
        <RepositoryListRow
          hasLocal={hasLocalRepositoryCheckout(repository, localRepoNames)}
          key={repository.repoAddress}
          onOpen={onOpen}
          onOpenTerminal={onOpenTerminal}
          profiles={profiles}
          project={project}
          repository={repository}
          summary={summaries?.[repository.repoAddress]}
        />
      ))}
    </div>
  );
}
