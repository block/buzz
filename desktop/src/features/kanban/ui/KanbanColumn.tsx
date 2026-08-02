import { Plus } from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { CardTile } from "@/features/kanban/ui/CardTile";
import type { KanbanCard } from "@/features/kanban/lib/kanbanTypes";
import type { KanbanColumn as KanbanColumnModel } from "@/features/kanban/lib/kanbanTypes";

type KanbanColumnProps = {
  column: KanbanColumnModel;
  cards: KanbanCard[];
  onCardClick: (card: KanbanCard) => void;
};

export function KanbanColumn({
  column,
  cards,
  onCardClick,
}: KanbanColumnProps) {
  return (
    <section
      className="flex w-72 shrink-0 flex-col rounded-lg border bg-muted/30"
      data-testid={`kanban-column-${column.id}`}
    >
      <header className="flex items-center justify-between gap-2 border-b px-3 py-2.5">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-semibold" title={column.name}>
            {column.name}
          </span>
          <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs tabular-nums text-muted-foreground">
            {cards.length}
          </span>
        </span>
        {column.wip !== null ? (
          <Badge
            className="shrink-0"
            title={`WIP limit ${column.wip}`}
            variant="secondary"
          >
            WIP {column.wip}
          </Badge>
        ) : null}
      </header>

      <div
        className="flex flex-col gap-2 overflow-y-auto p-2"
        data-testid={`kanban-column-cards-${column.id}`}
      >
        {cards.map((card) => (
          <CardTile card={card} key={card.id} onClick={onCardClick} />
        ))}

        {/* "+ New card" is disabled in P2; creation ships in P3. */}
        <Button
          className="w-full justify-start text-muted-foreground"
          disabled
          type="button"
          variant="ghost"
        >
          <Plus className="size-4" />
          New card
        </Button>
      </div>
    </section>
  );
}
