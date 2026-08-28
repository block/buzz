import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { LazyMeetingsRouteScreen } from "./lazyMeetingsRouteScreen";

type MeetingsRouteSearch = {
  room?: string;
  action?: "join" | "start";
};

function validateMeetingsSearch(
  search: Record<string, unknown>,
): MeetingsRouteSearch {
  const room =
    typeof search.room === "string" && search.room.length > 0
      ? search.room
      : undefined;
  const action =
    search.action === "join" || search.action === "start"
      ? search.action
      : undefined;
  return { room, action };
}

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
