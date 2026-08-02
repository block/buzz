import { useIdentityQuery } from "@/shared/api/hooks";
import {
  useBoardQuery,
  useCardsQuery,
} from "@/features/kanban/lib/boardQueries";
import type {
  KanbanBoard,
  KanbanCard,
} from "@/features/kanban/lib/kanbanTypes";
import { groupCardsByColumn } from "@/features/kanban/lib/kanbanTypes";

/**
 * Live board + card subscription for one board.
 *
 * Column renames and reorders (board 31001 updates) and card adds/moves
 * (card 31002 updates) all flow in over the live subscriptions wired inside
 * `useCardsQuery`, invalidating the TanStack queries so the view re-renders
 * with no polling and no manual refresh.
 */
export function useLiveBoard(boardId: string) {
  const { data: identity } = useIdentityQuery();
  const me = identity?.pubkey;

  const boardQuery = useBoardQuery(boardId, me);
  const board: KanbanBoard | null = boardQuery.data ?? null;
  const cardsQuery = useCardsQuery(board?.owner, boardId);
  const cards: KanbanCard[] = cardsQuery.data ?? [];

  return {
    board,
    cards,
    cardsByColumn: groupCardsByColumn(cards),
    isLoading: boardQuery.isLoading || cardsQuery.isLoading,
    isError: boardQuery.isError || cardsQuery.isError,
  };
}
