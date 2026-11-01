import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { parseProjectDetailSearch } from "@/features/projects/lib/projectDetailSearch";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ProjectDetailScreen = React.lazy(async () => {
  const module = await import("@/features/projects/ui/ProjectDetailScreen");
  return { default: module.ProjectDetailScreen };
});

export const Route = createFileRoute("/projects/$projectId")({
  component: ProjectDetailRouteComponent,
  validateSearch: parseProjectDetailSearch,
});

function ProjectDetailRouteComponent() {
  usePreviewFeatureWarning("projects");
  const { projectId } = Route.useParams();
  const {
    commitHash,
    filePath,
    homeTab,
    pullRequestId,
    issueId,
    repositoryId,
    tab,
  } = Route.useSearch();

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <ProjectDetailScreen
        commitHash={commitHash}
        filePath={filePath}
        homeTab={homeTab}
        issueId={issueId}
        projectId={projectId}
        pullRequestId={pullRequestId}
        repositoryId={repositoryId}
        tab={tab}
      />
    </React.Suspense>
  );
}
