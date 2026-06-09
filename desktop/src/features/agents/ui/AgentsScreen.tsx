import * as React from "react";

import { TopChromeBackdrop } from "@/shared/ui/TopChromeBackdrop";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const AgentsView = React.lazy(async () => {
  const module = await import("@/features/agents/ui/AgentsView");
  return { default: module.AgentsView };
});

export function AgentsScreen() {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <TopChromeBackdrop />
      <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
        <AgentsView />
      </React.Suspense>
    </div>
  );
}
