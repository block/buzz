import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

const CommandConsoleScreen = React.lazy(async () => {
  const module = await import(
    "@/features/command-console/ui/CommandConsoleScreen"
  );
  return { default: module.CommandConsoleScreen };
});

export const Route = createFileRoute("/console")({
  component: CommandConsoleRouteComponent,
});

function CommandConsoleRouteComponent() {
  return (
    <React.Suspense
      fallback={
        <div className="flex min-h-0 flex-1 items-center justify-center text-sm text-muted-foreground">
          Loading Command Console…
        </div>
      }
    >
      <CommandConsoleScreen />
    </React.Suspense>
  );
}
