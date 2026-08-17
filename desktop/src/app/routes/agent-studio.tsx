import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const AgentStudioScreen = React.lazy(async () => {
  const module = await import("@/features/agent-studio/ui/AgentStudioScreen");
  return { default: module.AgentStudioScreen };
});

export const Route = createFileRoute("/agent-studio")({
  component: AgentStudioRouteComponent,
});

function AgentStudioRouteComponent() {
  usePreviewFeatureWarning("agent-studio");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agent-studio" />}>
      <AgentStudioScreen />
    </React.Suspense>
  );
}
