import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const BattleRhythmScreen = React.lazy(async () => {
  const module = await import("@/features/battle-rhythm/ui/BattleRhythmScreen");
  return { default: module.BattleRhythmScreen };
});

export const Route = createFileRoute("/battle-rhythm")({
  component: BattleRhythmRoute,
});
function BattleRhythmRoute() {
  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="projects" />}
    >
      <BattleRhythmScreen />
    </React.Suspense>
  );
}
