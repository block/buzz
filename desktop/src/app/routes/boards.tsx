import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { BoardList } from "@/features/kanban/ui/BoardList";

export const Route = createFileRoute("/boards")({
  component: BoardsRouteComponent,
});

function BoardsRouteComponent() {
  return (
    <React.Suspense fallback={null}>
      <BoardList />
    </React.Suspense>
  );
}
