import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { Plus, Search } from "lucide-react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Badge } from "@/shared/ui/badge";
import { Skeleton } from "@/shared/ui/skeleton";
import {
  boardsQueryKey,
  useBoardsQuery,
  useMemberChannelIds,
} from "@/features/kanban/lib/boardQueries";
import { NewBoardDialog } from "@/features/kanban/ui/NewBoardDialog";
import type { KanbanBoard } from "@/features/kanban/lib/kanbanTypes";

type BoardScope = "all" | "owned" | "shared";

export function BoardList() {
  const { data: identity } = useIdentityQuery();
  const me = identity?.pubkey;
  const memberChannelIds = useMemberChannelIds();
  const boardsQuery = useBoardsQuery(me, memberChannelIds);
  const boards = boardsQuery.data ?? [];

  const [scope, setScope] = React.useState<BoardScope>("all");
  const [query, setQuery] = React.useState("");
  const [newBoardOpen, setNewBoardOpen] = React.useState(false);
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  async function handleBoardCreated(boardId: string) {
    await queryClient.invalidateQueries({ queryKey: boardsQueryKey() });
    navigate({ to: "/boards/$boardId", params: { boardId } });
  }

  const normalizedQuery = query.trim().toLowerCase();
  const filtered = boards.filter((board) => {
    const isOwned = me ? board.owner.toLowerCase() === me.toLowerCase() : false;
    if (scope === "owned" && !isOwned) return false;
    if (scope === "shared" && isOwned) return false;
    if (
      normalizedQuery.length > 0 &&
      !board.name.toLowerCase().includes(normalizedQuery)
    ) {
      return false;
    }
    return true;
  });

  return (
    <div className="mx-auto w-full max-w-4xl px-6 py-6">
      <header className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold">Boards</h1>
          <p className="mt-0.5 text-sm text-muted-foreground">
            Boards you own or were invited to.
          </p>
        </div>
        {/* In-app board creation (P3 write path) — signs under the Desktop identity. */}
        <Button
          disabled={!me}
          onClick={() => setNewBoardOpen(true)}
          type="button"
        >
          <Plus className="size-4" />
          New board
        </Button>
      </header>

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <div className="relative min-w-56 flex-1">
          <Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            className="pl-9"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search boards by title"
            value={query}
          />
        </div>
        <div className="flex items-center gap-1 rounded-lg border p-1">
          {(["all", "owned", "shared"] as const).map((option) => (
            <button
              className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                scope === option
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:text-foreground"
              }`}
              key={option}
              onClick={() => setScope(option)}
              type="button"
            >
              {option === "all"
                ? "All"
                : option === "owned"
                  ? "Owned by me"
                  : "Shared with me"}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-5">
        {boardsQuery.isLoading ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Skeleton className="h-28" />
            <Skeleton className="h-28" />
          </div>
        ) : filtered.length === 0 ? (
          <div className="rounded-lg border border-dashed p-10 text-center text-sm text-muted-foreground">
            {boards.length === 0
              ? "No boards yet. Create your first board."
              : "No boards match this filter."}
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            {filtered.map((board) => (
              <BoardTile
                board={board}
                isOwned={
                  me ? board.owner.toLowerCase() === me.toLowerCase() : false
                }
                key={board.id}
                onClick={() =>
                  navigate({
                    to: "/boards/$boardId",
                    params: { boardId: board.id },
                  })
                }
              />
            ))}
          </div>
        )}
      </div>

      {me ? (
        <NewBoardDialog
          onCreated={handleBoardCreated}
          onOpenChange={setNewBoardOpen}
          open={newBoardOpen}
          owner={me}
        />
      ) : null}
    </div>
  );
}

function BoardTile({
  board,
  isOwned,
  onClick,
}: {
  board: KanbanBoard;
  isOwned: boolean;
  onClick: () => void;
}) {
  const openCards = board.columns
    .map((column) => column.name)
    .filter((name) => name.toLowerCase() !== "done").length;

  return (
    <button
      className="group flex flex-col gap-2 rounded-lg border bg-card p-4 text-left shadow-sm transition-colors hover:border-border hover:bg-accent/40"
      data-testid={`board-tile-${board.id}`}
      onClick={onClick}
      type="button"
    >
      <span className="flex items-start justify-between gap-2">
        <span className="min-w-0">
          <span className="block truncate font-semibold" title={board.name}>
            {board.name}
          </span>
          {board.description ? (
            <span className="mt-0.5 line-clamp-2 block text-xs text-muted-foreground">
              {board.description}
            </span>
          ) : null}
        </span>
        <Badge className="shrink-0" variant={isOwned ? "default" : "secondary"}>
          {isOwned ? "Owned" : "Shared"}
        </Badge>
      </span>
      <span className="mt-auto flex items-center gap-3 text-xs text-muted-foreground">
        <span>{board.columns.length} columns</span>
        <span>{openCards} open</span>
      </span>
    </button>
  );
}
