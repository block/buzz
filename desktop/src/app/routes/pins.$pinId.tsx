import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Navigate, createFileRoute } from "@tanstack/react-router";

import { PinnedSiteScreen } from "@/features/pinned-sites/ui/PinnedSiteScreen";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export const Route = createFileRoute("/pins/$pinId")({
  component: PinnedSiteRouteComponent,
});

function PinnedSiteRouteComponent() {
  const { pinId } = Route.useParams();
  if (!isTauri()) return <Navigate to="/" replace />; // pinned sites do not exist on web
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      {/* key: a pin switch must remount with fresh per-pin state — never reuse
          the previous pin's startUrl for the next pin's webview. */}
      <PinnedSiteScreen key={pinId} pinId={pinId} />
    </React.Suspense>
  );
}
