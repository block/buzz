import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const TriageScreen = React.lazy(async () => {
  const module = await import("@/features/triage/ui/TriageScreen");
  return { default: module.TriageScreen };
});

export const Route = createFileRoute("/triage")({
  component: TriageRouteComponent,
});

function TriageRouteComponent() {
  usePreviewFeatureWarning("triage");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="triage" />}>
      <TriageScreen />
    </React.Suspense>
  );
}
