import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import { KIND_KANBAN_BOARD, KIND_KANBAN_CARD } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";
import {
  collapseBoards,
  collapseCards,
  type KanbanBoard,
  type KanbanCard,
} from "@/features/kanban/lib/kanbanTypes";

export const boardsQueryKey = () => ["kanban", "boards"] as const;
export const boardQueryKey = (boardId: string) =>
  ["kanban", "board", boardId] as const;
export const cardsQueryKey = (boardRef: string) =>
  ["kanban", "cards", boardRef] as const;

const BOARD_SCAN_LIMIT = 500;
const CARD_SCAN_LIMIT = 1_000;

function samePubkey(left: string, right: string): boolean {
  return left.toLowerCase() === right.toLowerCase();
}

function boardIsAccessible(
  board: KanbanBoard,
  me: string | undefined,
): boolean {
  if (!me) return false;
  if (samePubkey(board.owner, me)) return true;
  return board.invites.some((invite) => samePubkey(invite, me));
}

/**
 * Fetch every board head currently on the relay, then narrow client-side to
 * the ones the current user can see: boards they own plus boards whose
 * `invite` tag names them. Channel-shared boards (`h` tags) may not be found
 * by a plain `{kinds:[31001]}` scan because the relay index is
 * author/kind-based and may not index custom tags — see the P2 review flag in
 * the build notes. Scheduling a follow-up `#invite`-indexed REQ is a P3+
 * improvement once relay indexing of `invite` is confirmed.
 */
async function fetchBoards(me: string | undefined): Promise<KanbanBoard[]> {
  if (!me) return [];
  const events = await relayClient.fetchEvents({
    kinds: [KIND_KANBAN_BOARD],
    limit: BOARD_SCAN_LIMIT,
  });
  return collapseBoards(events).filter((board) => boardIsAccessible(board, me));
}

async function fetchBoard(
  boardId: string,
  me: string | undefined,
): Promise<KanbanBoard | null> {
  if (!me) return null;
  const events = await relayClient.fetchEvents({
    kinds: [KIND_KANBAN_BOARD],
    "#d": [boardId],
    limit: 20,
  });
  const boards = collapseBoards(events);
  return boards.find((board) => boardIsAccessible(board, me)) ?? null;
}

async function fetchCards(
  boardOwner: string | undefined,
  boardId: string | undefined,
): Promise<KanbanCard[]> {
  if (!boardOwner || !boardId) return [];
  // Same `#a` REQ filter pattern the CLI `issues list` uses.
  const boardRef = `31001:${boardOwner}:${boardId}`;
  const events: RelayEvent[] = await relayClient.fetchEvents({
    kinds: [KIND_KANBAN_CARD],
    "#a": [boardRef],
    limit: CARD_SCAN_LIMIT,
  });
  return collapseCards(events);
}

function useBoardLiveUpdates(
  boardId: string | undefined,
  boardRef: string | undefined,
): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    if (!boardId) return;
    let disposed = false;
    let disposeBoard: (() => Promise<void>) | null = null;
    let disposeCards: (() => Promise<void>) | null = null;

    const subscribe = (filter: RelaySubscriptionFilter) =>
      relayClient.subscribeLive(filter, () => {
        void queryClient.invalidateQueries({
          queryKey: ["kanban"],
        });
      });

    void subscribe({ kinds: [KIND_KANBAN_BOARD], "#d": [boardId], limit: 0 })
      .then((unsubscribe) => {
        if (disposed) {
          void unsubscribe();
        } else {
          disposeBoard = unsubscribe;
        }
      })
      .catch((error) => {
        console.error("Couldn’t subscribe to board updates", error);
      });

    if (boardRef) {
      void subscribe({ kinds: [KIND_KANBAN_CARD], "#a": [boardRef], limit: 0 })
        .then((unsubscribe) => {
          if (disposed) {
            void unsubscribe();
          } else {
            disposeCards = unsubscribe;
          }
        })
        .catch((error) => {
          console.error("Couldn’t subscribe to card updates", error);
        });
    }

    const unsubscribeReconnect = relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({ queryKey: ["kanban"] });
    });

    return () => {
      disposed = true;
      unsubscribeReconnect();
      if (disposeBoard) void disposeBoard();
      if (disposeCards) void disposeCards();
    };
  }, [boardId, boardRef, queryClient]);
}

/** Every board the current user can see (own + invited). */
export function useBoardsQuery(me: string | undefined) {
  return useQuery({
    enabled: Boolean(me),
    queryKey: boardsQueryKey(),
    queryFn: () => fetchBoards(me),
    staleTime: 15_000,
  });
}

/** A single board head by id. */
export function useBoardQuery(boardId: string, me: string | undefined) {
  return useQuery({
    enabled: Boolean(me),
    queryKey: boardQueryKey(boardId),
    queryFn: () => fetchBoard(boardId, me),
    staleTime: 30_000,
  });
}

/** All cards for a board, keyed by its `a` ref. */
export function useCardsQuery(
  boardOwner: string | undefined,
  boardId: string | undefined,
) {
  const boardRef =
    boardOwner && boardId ? `31001:${boardOwner}:${boardId}` : undefined;
  useBoardLiveUpdates(boardId, boardRef);
  return useQuery({
    enabled: Boolean(boardRef),
    queryKey: cardsQueryKey(boardRef ?? ""),
    queryFn: () => fetchCards(boardOwner, boardId),
    staleTime: 15_000,
  });
}
