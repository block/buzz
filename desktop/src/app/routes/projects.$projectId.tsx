import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { validateProjectDetailSearch } from "@/features/projects/lib/projectDetailSearch";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ProjectDetailScreen = React.lazy(async () => {
  const module = await import("@/features/projects/ui/ProjectDetailScreen");
  return { default: module.ProjectDetailScreen };
});

export const Route = createFileRoute("/projects/$projectId")({
  component: ProjectDetailRouteComponent,
  validateSearch: validateProjectDetailSearch,
});

function ProjectDetailRouteComponent() {
  usePreviewFeatureWarning("projects");
  const { projectId } = Route.useParams();
  const {
    commitHash,
    pullRequestId,
    issueId,
    repositoryId,
    repoRef,
    repoPath,
  } = Route.useSearch();

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <ProjectDetailScreen
        commitHash={commitHash}
        issueId={issueId}
        projectId={projectId}
        pullRequestId={pullRequestId}
        repoPath={repoPath}
        repoRef={repoRef}
        repositoryId={repositoryId}
      />
    </React.Suspense>
  );
}
