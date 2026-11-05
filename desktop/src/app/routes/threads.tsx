import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ThreadsScreen = React.lazy(async () => {
  const module = await import("@/features/threads/ui/ThreadsScreen");
  return { default: module.ThreadsScreen };
});

export const Route = createFileRoute("/threads")({
  component: ThreadsRouteComponent,
});

function ThreadsRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <ThreadsScreen />
    </React.Suspense>
  );
}
