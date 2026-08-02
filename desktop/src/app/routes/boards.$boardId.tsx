import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { BoardView } from "@/features/kanban/ui/BoardView";

export const Route = createFileRoute("/boards/$boardId")({
  component: BoardIdRouteComponent,
});

function BoardIdRouteComponent() {
  const { boardId } = Route.useParams();
  return (
    <React.Suspense fallback={null}>
      <BoardView boardId={boardId} />
    </React.Suspense>
  );
}
