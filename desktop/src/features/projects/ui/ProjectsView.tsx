import { ExternalLink, FolderGit2, GitFork, Users } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useProjectsQuery } from "@/features/projects/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";

function EmptyState() {
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

export function ProjectsView() {
  const { goProject } = useAppNavigation();
  const projectsQuery = useProjectsQuery();
  const projects = projectsQuery.data ?? [];

  if (projectsQuery.isLoading) {
    return null;
  }

  if (projectsQuery.isError) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
        <p className="text-sm text-red-400">Failed to load projects</p>
        <Button
          onClick={() => void projectsQuery.refetch()}
          size="sm"
          variant="outline"
        >
          Retry
        </Button>
      </div>
    );
  }

  if (projects.length === 0) {
    return <EmptyState />;
  }

  return (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4",
        topChromeInset.padding,
      )}
    >
      <div className="mb-3 flex items-center gap-2">
        <h2 className="text-sm font-medium text-muted-foreground">
          {projects.length} {projects.length === 1 ? "project" : "projects"}
        </h2>
      </div>

      <div className="space-y-2">
        {projects.map((project) => (
          <Card
            className="relative p-4 transition-colors hover:bg-muted/50"
            key={project.id}
          >
            <button
              className="absolute inset-0 rounded-lg"
              onClick={() => {
                void goProject(project.dtag);
              }}
              type="button"
            >
              <span className="sr-only">View {project.name}</span>
            </button>
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 flex-1 space-y-1.5">
                <div className="flex items-center gap-2">
                  <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="truncate text-sm font-semibold">
                    {project.name}
                  </span>
                </div>
                {project.description ? (
                  <p className="line-clamp-2 text-sm text-muted-foreground">
                    {project.description}
                  </p>
                ) : null}
                <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground/70">
                  {project.cloneUrls.length > 0 ? (
                    <span className="flex items-center gap-1">
                      <GitFork className="h-4 w-4" />
                      {project.cloneUrls[0]}
                    </span>
                  ) : null}
                  {project.contributors.length > 0 ? (
                    <span className="flex items-center gap-1">
                      <Users className="h-4 w-4" />
                      {project.contributors.length}
                    </span>
                  ) : null}
                  {project.webUrl ? (
                    <span className="flex items-center gap-1">
                      <ExternalLink className="h-4 w-4" />
                      Web
                    </span>
                  ) : null}
                </div>
              </div>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
