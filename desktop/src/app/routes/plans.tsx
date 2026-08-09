import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const PlansScreen = React.lazy(async () => {
  const module = await import("@/features/plans/ui/PlansScreen");
  return { default: module.PlansScreen };
});

export const Route = createFileRoute("/plans")({
  component: PlansRoute,
});

function PlansRoute() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <PlansScreen />
    </React.Suspense>
  );
}
