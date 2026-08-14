import { createRootRoute } from "@tanstack/react-router";

import { AppShell } from "@/app/AppShell";
import { NativeTeamSnapshotBridge } from "@/features/agents/NativeTeamSnapshotBridge";

export const Route = createRootRoute({
  component: RootRoute,
});

function RootRoute() {
  return (
    <>
      <NativeTeamSnapshotBridge />
      <AppShell />
    </>
  );
}
