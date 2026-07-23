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
    commitHash:
      typeof search.commitHash === "string" ? search.commitHash : undefined,
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
  const { commitHash, pullRequestId, issueId } = Route.useSearch();

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <ProjectDetailScreen
        commitHash={commitHash}
        issueId={issueId}
        projectId={projectId}
        pullRequestId={pullRequestId}
      />
    </React.Suspense>
  );
}
