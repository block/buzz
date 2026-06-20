import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export const Route = createFileRoute("/workflows/$workflowId")({
  component: WorkflowDetailRouteComponent,
});

const WorkflowsRouteScreen = React.lazy(async () => {
  const module = await import("./WorkflowsRouteScreen");
  return { default: module.WorkflowsRouteScreen };
});

function WorkflowDetailRouteComponent() {
  usePreviewFeatureWarning("workflows");
  const { workflowId } = Route.useParams();

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <WorkflowsRouteScreen selectedWorkflowId={workflowId} />
    </React.Suspense>
  );
}
