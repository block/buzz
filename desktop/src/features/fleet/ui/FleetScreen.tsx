import * as React from "react";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const FleetView = React.lazy(async () => {
  const module = await import("@/features/fleet/ui/FleetView");
  return { default: module.FleetView };
});

export function FleetScreen() {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
        <FleetView />
      </React.Suspense>
    </div>
  );
}
