import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const SavedScreen = React.lazy(async () => {
  const module = await import("@/features/bookmarks/ui/SavedScreen");
  return { default: module.SavedScreen };
});

export const Route = createFileRoute("/saved")({
  component: SavedRouteComponent,
});

function SavedRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <SavedScreen />
    </React.Suspense>
  );
}
