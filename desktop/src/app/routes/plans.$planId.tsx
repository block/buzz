import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const PlanDetailScreen = React.lazy(async () => {
  const module = await import("@/features/plans/ui/PlanDetailScreen");
  return { default: module.PlanDetailScreen };
});

export const Route = createFileRoute("/plans/$planId")({
  component: PlanDetailRoute,
  validateSearch: (search: Record<string, unknown>) => ({
    task: typeof search.task === "string" ? search.task : undefined,
  }),
});

function PlanDetailRoute() {
  const { planId } = Route.useParams();
  const { task } = Route.useSearch();
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
      <PlanDetailScreen planId={planId} selectedTaskId={task} />
    </React.Suspense>
  );
}
