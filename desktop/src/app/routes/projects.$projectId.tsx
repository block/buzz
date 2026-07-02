import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ProjectDetailScreen = React.lazy(async () => {
  const module = await import("@/features/projects/ui/ProjectDetailScreen");
  return { default: module.ProjectDetailScreen };
});

export const Route = createFileRoute("/projects/$projectId")({
  component: ProjectDetailRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    pullRequestId:
      typeof search.pullRequestId === "string"
        ? search.pullRequestId
        : undefined,
    issueId: typeof search.issueId === "string" ? search.issueId : undefined,
  }),
});

function ProjectDetailRouteComponent() {
  usePreviewFeatureWarning("projects");
  const { projectId } = Route.useParams();
  const { pullRequestId, issueId } = Route.useSearch();

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <ProjectDetailScreen
        issueId={issueId}
        projectId={projectId}
        pullRequestId={pullRequestId}
      />
    </React.Suspense>
  );
}
