import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const BuzzTermPage = React.lazy(async () => {
  const module = await import("@/features/bridge/ui/BuzzTermPage");
  return { default: module.BuzzTermPage };
});

export const Route = createFileRoute("/buzz-term")({
  component: BuzzTermRouteComponent,
});

function BuzzTermRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <BuzzTermPage />
    </React.Suspense>
  );
}
