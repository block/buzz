import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const FleetScreen = React.lazy(async () => {
  const module = await import("@/features/fleet/ui/FleetScreen");
  return { default: module.FleetScreen };
});

export const Route = createFileRoute("/fleet")({
  component: FleetRouteComponent,
});

function FleetRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <FleetScreen />
    </React.Suspense>
  );
}
