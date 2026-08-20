import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const WorkstreamBoardScreen = React.lazy(async () => {
  const module = await import(
    "@/features/workstream-board/ui/WorkstreamBoardScreen"
  );
  return { default: module.WorkstreamBoardScreen };
});

export const Route = createFileRoute("/workstreams")({
  component: WorkstreamsRouteComponent,
});

function WorkstreamsRouteComponent() {
  usePreviewFeatureWarning("workstreamBoard");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <WorkstreamBoardScreen />
    </React.Suspense>
  );
}
