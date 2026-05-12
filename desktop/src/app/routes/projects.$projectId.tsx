import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ProjectDetailScreen = React.lazy(async () => {
  const module = await import("@/features/projects/ui/ProjectDetailScreen");
  return { default: module.ProjectDetailScreen };
});

export const Route = createFileRoute("/projects/$projectId")({
  component: ProjectDetailRouteComponent,
});

function ProjectDetailRouteComponent() {
  const { projectId } = Route.useParams();

  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="projects" />}
    >
      <ProjectDetailScreen projectId={projectId} />
    </React.Suspense>
  );
}
