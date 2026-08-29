import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { validateMeetingsSearch } from "@/features/meetings/ui/meetingsRouteSearch";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { LazyMeetingsRouteScreen } from "./lazyMeetingsRouteScreen";

export const Route = createFileRoute("/meetings")({
  component: MeetingsRouteComponent,
  validateSearch: validateMeetingsSearch,
});

function MeetingsRouteComponent() {
  usePreviewFeatureWarning("meetings");
  const { action, room } = Route.useSearch();
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="meetings" />}>
      <LazyMeetingsRouteScreen action={action} room={room} />
    </React.Suspense>
  );
}
