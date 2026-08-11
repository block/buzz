import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const AttentionScreen = React.lazy(async () => {
  const module = await import("@/features/attention/ui/AttentionScreen");
  return { default: module.AttentionScreen };
});

export const Route = createFileRoute("/attention")({
  component: AttentionRouteComponent,
});

function AttentionRouteComponent() {
  usePreviewFeatureWarning("attention");
  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="pulse" />}
    >
      <AttentionScreen />
    </React.Suspense>
  );
}
