import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const LivingShipScreen = React.lazy(async () => {
  const module = await import("@/features/living-ship/ui/LivingShipScreen");
  return { default: module.LivingShipScreen };
});

export const Route = createFileRoute("/ship")({
  component: LivingShipRouteComponent,
});

function LivingShipRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <LivingShipScreen />
    </React.Suspense>
  );
}
