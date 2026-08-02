import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, Share2, SlidersHorizontal } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { useLiveBoard } from "@/features/kanban/lib/useLiveBoard";
import { KanbanColumn } from "@/features/kanban/ui/KanbanColumn";
import { CardDetailModal } from "@/features/kanban/ui/CardDetailModal";
import type { KanbanCard } from "@/features/kanban/lib/kanbanTypes";

type BoardViewProps = {
  boardId: string;
};

export function BoardView({ boardId }: BoardViewProps) {
  const navigate = useNavigate();
  const { board, cardsByColumn, isLoading } = useLiveBoard(boardId);
  const [detailCard, setDetailCard] = React.useState<KanbanCard | null>(null);

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-2 border-b px-4 py-2.5">
        <Button
          aria-label="Back to all boards"
          onClick={() => navigate({ to: "/boards" })}
          size="icon"
          type="button"
          variant="ghost"
        >
          <ArrowLeft className="size-4" />
        </Button>
        <div className="min-w-0 flex-1">
          {isLoading ? (
            <Skeleton className="h-6 w-40" />
          ) : (
            <h1 className="truncate text-base font-semibold">
              {board?.name ?? "Board"}
            </h1>
          )}
        </div>
        <Button
          className="text-muted-foreground"
          disabled
          title="Sharing ships in P3"
          type="button"
          variant="ghost"
        >
          <Share2 className="size-4" />
          Share
        </Button>
        <Button
          className="text-muted-foreground"
          disabled
          title="Filter ships in P4"
          type="button"
          variant="ghost"
        >
          <SlidersHorizontal className="size-4" />
          Filter
        </Button>
      </header>

      {isLoading ? (
        <div className="flex flex-1 items-center justify-center p-8">
          <Skeleton className="h-32 w-72" />
        </div>
      ) : !board ? (
        <div className="flex flex-1 items-center justify-center p-8 text-sm text-muted-foreground">
          Board not found, or you don&apos;t have access to it.
        </div>
      ) : (
        <div className="flex flex-1 gap-3 overflow-x-auto p-4">
          {board.columns.map((column) => (
            <KanbanColumn
              cards={cardsByColumn.get(column.id) ?? []}
              column={column}
              key={column.id}
              onCardClick={setDetailCard}
            />
          ))}
          {board.columns.length === 0 ? (
            <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
              This board has no columns yet.
            </div>
          ) : null}
        </div>
      )}

      <CardDetailModal
        card={detailCard}
        onOpenChange={(open) => {
          if (!open) setDetailCard(null);
        }}
      />
    </div>
  );
}
