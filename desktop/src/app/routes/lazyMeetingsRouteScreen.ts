import * as React from "react";

export const LazyMeetingsRouteScreen = React.lazy(async () => {
  const module = await import("./MeetingsRouteScreen");
  return { default: module.MeetingsRouteScreen };
});
