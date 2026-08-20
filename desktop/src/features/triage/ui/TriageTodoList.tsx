import { Check, ExternalLink, X } from "lucide-react";

import type { TriageTodo } from "@/features/triage/api";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

type TriageTodoListProps = {
  isLoading: boolean;
  onComplete: (todo: TriageTodo) => void;
  onDismiss: (todo: TriageTodo) => void;
  onOpenThread: (todo: TriageTodo) => void;
  todos: readonly TriageTodo[];
};

export function TriageTodoList({
  isLoading,
  onComplete,
  onDismiss,
  onOpenThread,
  todos,
}: TriageTodoListProps) {
  if (isLoading) {
    return (
      <div className="space-y-3 p-4">
        {["one", "two", "three"].map((row) => (
          <div className="space-y-1.5" key={row}>
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-3 w-2/3" />
          </div>
        ))}
      </div>
    );
  }

  const open = todos.filter((todo) => todo.status === "open");
  const resolved = todos.filter((todo) => todo.status !== "open");

  if (todos.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="max-w-sm text-center text-sm text-muted-foreground">
          Nothing adopted yet. Add an item from Important and it shows up here
          as a todo, with the reason it mattered.
        </p>
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-2 p-4">
      {[...open, ...resolved].map((todo) => (
        <div
          className={cn(
            "rounded-lg border border-border/60 p-3",
            todo.status !== "open" && "opacity-60",
          )}
          data-testid="triage-todo"
          key={todo.id}
        >
          <div className="flex min-w-0 items-center gap-2">
            {todo.authorLabel ? (
              <span className="min-w-0 truncate text-sm font-medium text-foreground">
                {todo.authorLabel}
              </span>
            ) : null}
            {todo.channelName ? (
              <Badge variant="outline">#{todo.channelName}</Badge>
            ) : null}
            {todo.status === "done" ? (
              <Badge variant="success">Done</Badge>
            ) : null}
            {todo.status === "dismissed" ? (
              <Badge variant="secondary">Dismissed</Badge>
            ) : null}
          </div>

          {todo.preview.trim() ? (
            <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
              {todo.preview}
            </p>
          ) : null}

          {todo.reason ? (
            <p className="mt-1.5 text-2xs italic text-muted-foreground/80">
              {todo.reason}
            </p>
          ) : null}

          <div className="mt-2.5 flex flex-wrap gap-2">
            {todo.channelId ? (
              <Button
                onClick={() => onOpenThread(todo)}
                size="sm"
                type="button"
                variant="outline"
              >
                <ExternalLink className="h-4 w-4" />
                Open thread
              </Button>
            ) : null}
            {todo.status === "open" ? (
              <>
                <Button
                  data-testid="triage-todo-done"
                  onClick={() => onComplete(todo)}
                  size="sm"
                  type="button"
                >
                  <Check className="h-4 w-4" />
                  Done
                </Button>
                <Button
                  onClick={() => onDismiss(todo)}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                  Dismiss
                </Button>
              </>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}
