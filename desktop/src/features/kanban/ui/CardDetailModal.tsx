import { CalendarDays, User } from "lucide-react";

import { Markdown } from "@/shared/ui/markdown";
import { Badge } from "@/shared/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Avatar, AvatarFallback } from "@/shared/ui/avatar";
import type { KanbanCard } from "@/features/kanban/lib/kanbanTypes";

type CardDetailModalProps = {
  card: KanbanCard | null;
  onOpenChange: (open: boolean) => void;
};

/**
 * Read-only card detail (P2). Markdown body + labels + assignees + due.
 * Comments and reactions on the card thread arrive in P4.
 */
export function CardDetailModal({ card, onOpenChange }: CardDetailModalProps) {
  return (
    <Dialog open={card !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="text-base">{card?.title}</DialogTitle>
          <DialogDescription asChild>
            <span className="sr-only">Kanban card detail</span>
          </DialogDescription>
        </DialogHeader>

        {card ? (
          <div className="space-y-4 text-sm">
            {card.labels.length > 0 ? (
              <div className="flex flex-wrap items-center gap-1.5">
                {card.labels.map((label) => (
                  <Badge key={label} variant="outline">
                    {label}
                  </Badge>
                ))}
              </div>
            ) : null}

            <div className="max-h-[50vh] overflow-y-auto">
              <Markdown className="text-sm" content={card.content} />
            </div>

            <div className="flex items-center justify-between gap-3 border-t pt-3">
              <span className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">Assignees</span>
                <span className="flex -space-x-1.5">
                  {card.assignees.length === 0 ? (
                    <span className="text-xs text-muted-foreground/70">
                      None
                    </span>
                  ) : (
                    card.assignees.map((assignee) => (
                      <Avatar
                        className="size-6 border-2 border-background"
                        key={assignee}
                      >
                        <AvatarFallback>
                          <User className="size-3.5" />
                        </AvatarFallback>
                      </Avatar>
                    ))
                  )}
                </span>
              </span>
              {card.due ? (
                <span
                  className="inline-flex items-center gap-1 text-xs text-muted-foreground"
                  title={`Due ${card.due}`}
                >
                  <CalendarDays className="size-3.5" />
                  {card.due}
                </span>
              ) : null}
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
