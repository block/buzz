import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

// The chunk import is hoisted so it can be triggered eagerly (route preload)
// as well as lazily on render — calling it twice is a no-op, the module loader
// dedupes and caches the in-flight promise.
const importAgentsScreen = () => import("@/features/agents/ui/AgentsScreen");

const AgentsScreen = React.lazy(async () => {
  const module = await importAgentsScreen();
  return { default: module.AgentsScreen };
});

/** Warms the AgentsScreen route chunk so first navigation doesn't stall. */
export function preloadAgentsScreen(): void {
  void importAgentsScreen();
}

export const Route = createFileRoute("/agents")({
  component: AgentsRouteComponent,
});

function AgentsRouteComponent() {
  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="agents" />}
    >
      <AgentsScreen />
    </React.Suspense>
  );
}
