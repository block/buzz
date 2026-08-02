import { CalendarDays, User } from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Avatar, AvatarFallback } from "@/shared/ui/avatar";
import { cn } from "@/shared/lib/cn";
import type { KanbanCard } from "@/features/kanban/lib/kanbanTypes";

type CardTileProps = {
  card: KanbanCard;
  onClick: (card: KanbanCard) => void;
};

export function CardTile({ card, onClick }: CardTileProps) {
  return (
    <Button
      asChild={false}
      className={cn(
        "h-auto w-full flex-col items-stretch gap-2 rounded-lg border bg-card p-3 text-left shadow-sm transition-colors",
        "hover:border-border hover:bg-accent/40",
      )}
      onClick={() => onClick(card)}
      type="button"
      variant="ghost"
    >
      <span className="line-clamp-2 text-sm font-medium text-foreground">
        {card.title}
      </span>

      {card.labels.length > 0 ? (
        <span className="flex flex-wrap items-center gap-1">
          {card.labels.map((label) => (
            <Badge className="inline-flex" key={label} variant="outline">
              {label}
            </Badge>
          ))}
        </span>
      ) : null}

      {card.assignees.length > 0 || card.due ? (
        <span className="flex items-center justify-between gap-2">
          <span className="flex -space-x-1.5">
            {card.assignees.slice(0, 3).map((assignee) => (
              <Avatar className="size-5 border-2 border-card" key={assignee}>
                <AvatarFallback>
                  <User className="size-3" />
                </AvatarFallback>
              </Avatar>
            ))}
            {card.assignees.length > 3 ? (
              <Avatar className="size-5 border-2 border-card">
                <AvatarFallback>
                  <User className="size-3" />
                </AvatarFallback>
              </Avatar>
            ) : null}
          </span>
          {card.due ? (
            <span
              className="inline-flex items-center gap-1 text-2xs text-muted-foreground"
              title={`Due ${card.due}`}
            >
              <CalendarDays className="size-3" />
              {card.due}
            </span>
          ) : null}
        </span>
      ) : null}
    </Button>
  );
}
