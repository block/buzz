import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const PulseScreen = React.lazy(async () => {
  const module = await import("@/features/pulse/ui/PulseScreen");
  return { default: module.PulseScreen };
});

export const Route = createFileRoute("/pulse")({
  component: PulseRouteComponent,
});

function PulseRouteComponent() {
  usePreviewFeatureWarning("pulse");
  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="pulse" />}
    >
      <PulseScreen />
    </React.Suspense>
  );
}
