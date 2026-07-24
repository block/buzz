import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const GroupsScreen = React.lazy(async () => {
  const module = await import("@/features/groups/ui/GroupsScreen");
  return { default: module.GroupsScreen };
});

export const Route = createFileRoute("/groups")({
  component: GroupsRouteComponent,
});

function GroupsRouteComponent() {
  usePreviewFeatureWarning("groups");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="groups" />}>
      <GroupsScreen />
    </React.Suspense>
  );
}
