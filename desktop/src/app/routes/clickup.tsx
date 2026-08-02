import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ClickUpScreen = React.lazy(async () => {
  const module = await import("@/features/clickup/ui/ClickUpScreen");
  return { default: module.ClickUpScreen };
});

export const Route = createFileRoute("/clickup")({
  component: ClickUpRouteComponent,
});

function ClickUpRouteComponent() {
  usePreviewFeatureWarning("clickup");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="clickup" />}>
      <ClickUpScreen />
    </React.Suspense>
  );
}
