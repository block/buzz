import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const VoiceModeScreen = React.lazy(async () => {
  const module = await import("@/features/agents/ui/VoiceModeScreen");
  return { default: module.VoiceModeScreen };
});

export const Route = createFileRoute("/voice")({
  component: VoiceRouteComponent,
});

function VoiceRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <VoiceModeScreen />
    </React.Suspense>
  );
}
