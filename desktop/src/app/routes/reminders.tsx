import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

const RemindersScreen = React.lazy(async () => {
  const module = await import("@/features/reminders/ui/RemindersScreen");
  return { default: module.RemindersScreen };
});

export const Route = createFileRoute("/reminders")({
  component: RemindersRouteComponent,
});

function RemindersRouteComponent() {
  return (
    <React.Suspense fallback={null}>
      <RemindersScreen />
    </React.Suspense>
  );
}
