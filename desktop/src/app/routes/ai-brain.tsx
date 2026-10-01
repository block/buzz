import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const BrainView = React.lazy(async () => {
  const module = await import("@/features/memory/ui/BrainView");
  return { default: module.BrainView };
});

export const Route = createFileRoute("/ai-brain")({
  component: AiBrainRouteComponent,
});

function AiBrainRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <BrainView />
    </React.Suspense>
  );
}
