import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import {
  MEETING_ROOM_NAME_MAX,
  MEETING_ROOM_NAME_MIN,
  normalizeMeetingRoomName,
} from "@/features/meetings/ui/meetingRoomName";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { LazyMeetingsRouteScreen } from "./lazyMeetingsRouteScreen";

type MeetingsRouteSearch = {
  room?: string;
  action?: "join" | "start";
};

function validateMeetingsSearch(
  search: Record<string, unknown>,
): MeetingsRouteSearch {
  // Normalize + bounds-check the deep-link room the same way the start form
  // does, so a hand-edited or stale URL can't push an unsanitized name into the
  // register/join flow. Drop it entirely when it doesn't survive.
  const normalizedRoom =
    typeof search.room === "string"
      ? normalizeMeetingRoomName(search.room)
          .slice(0, MEETING_ROOM_NAME_MAX)
          .replace(/[-_]+$/, "")
      : "";
  const room =
    normalizedRoom.length >= MEETING_ROOM_NAME_MIN ? normalizedRoom : undefined;
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
