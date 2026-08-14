import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export const Route = createFileRoute("/workflows")({
  component: WorkflowsRouteComponent,
});

const WorkflowsRouteScreen = React.lazy(async () => {
  const module = await import("./WorkflowsRouteScreen");
  return { default: module.WorkflowsRouteScreen };
});

function WorkflowsRouteComponent() {
  usePreviewFeatureWarning("workflows");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <WorkflowsRouteScreen selectedWorkflowId={null} />
    </React.Suspense>
  );
}
