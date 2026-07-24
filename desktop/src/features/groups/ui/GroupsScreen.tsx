import * as React from "react";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const GroupsView = React.lazy(async () => {
  const module = await import("./GroupsView");
  return { default: module.GroupsView };
});

export function GroupsScreen() {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <React.Suspense fallback={<ViewLoadingFallback kind="groups" />}>
        <GroupsView />
      </React.Suspense>
    </div>
  );
}
