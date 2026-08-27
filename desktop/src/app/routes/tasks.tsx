import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const TasksScreen = React.lazy(async () => {
  const module = await import("@/features/tasks/ui/TasksScreen");
  return { default: module.TasksScreen };
});

export const Route = createFileRoute("/tasks")({
  component: TasksRouteComponent,
});

function TasksRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="tasks" />}>
      <TasksScreen />
    </React.Suspense>
  );
}
