import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const FlowStudioScreen = React.lazy(async () => {
  const module = await import("@/features/flow-studio/ui/FlowStudioScreen");
  return { default: module.FlowStudioScreen };
});

export const Route = createFileRoute("/flow-studio")({
  component: FlowStudioRouteComponent,
});

function FlowStudioRouteComponent() {
  usePreviewFeatureWarning("flow-studio");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="flow-studio" />}>
      <FlowStudioScreen />
    </React.Suspense>
  );
}
