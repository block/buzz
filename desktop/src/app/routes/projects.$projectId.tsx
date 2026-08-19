import * as React from "react";
import { createFileRoute, useLocation } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { isEntityLinkTab } from "@/shared/lib/entityLink";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ProjectDetailScreen = React.lazy(async () => {
  const module = await import("@/features/projects/ui/ProjectDetailScreen");
  return { default: module.ProjectDetailScreen };
});

export const Route = createFileRoute("/projects/$projectId")({
  component: ProjectDetailRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    commitHash:
      typeof search.commitHash === "string" ? search.commitHash : undefined,
    pullRequestId:
      typeof search.pullRequestId === "string"
        ? search.pullRequestId
        : undefined,
    issueId: typeof search.issueId === "string" ? search.issueId : undefined,
    repositoryId:
      typeof search.repositoryId === "string" ? search.repositoryId : undefined,
    tab: isEntityLinkTab(search.tab) ? search.tab : undefined,
  }),
});

function ProjectDetailRouteComponent() {
  usePreviewFeatureWarning("projects");
  const { projectId } = Route.useParams();
  const { commitHash, pullRequestId, issueId, repositoryId, tab } =
    Route.useSearch();
  const entityNavigationId = useLocation({
    select: (location) => {
      const value = (
        location.state as { entityNavigationId?: unknown } | undefined
      )?.entityNavigationId;
      return typeof value === "string" ? value : undefined;
    },
  });

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <ProjectDetailScreen
        commitHash={commitHash}
        entityNavigationId={entityNavigationId}
        issueId={issueId}
        projectId={projectId}
        pullRequestId={pullRequestId}
        repositoryId={repositoryId}
        tab={tab}
      />
    </React.Suspense>
  );
}
