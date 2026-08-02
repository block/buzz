import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import { KIND_KANBAN_BOARD, KIND_KANBAN_CARD } from "@/shared/constants/kinds";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { RelayEvent } from "@/shared/api/types";
import {
  boardIsAccessible,
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

/**
 * Ids of the channels the current user is a member of. Used to resolve
 * channel-shared (`h`) board visibility — a board shared to a channel you
 * belong to is readable.
 */
export function useMemberChannelIds(): string[] {
  const { data: channels } = useChannelsQuery();
  return (channels ?? [])
    .filter((channel) => channel.isMember === true)
    .map((channel) => channel.id);
}

const BOARD_SCAN_LIMIT = 500;
const CARD_SCAN_LIMIT = 1_000;

/**
 * Fetch every board head currently on the relay, then narrow client-side to
 * the ones the current user can see: boards they own, boards whose `invite`
 * tag names them, and boards shared to a channel they are a member of (`h`).
 * The P3 entry-gate confirmed the bare `{kinds:[31001]}` scan returns ALL
 * community boards (global-only, ungated), so this filter is the access
 * boundary — keep it authoritative.
 */
async function fetchBoards(
  me: string | undefined,
  memberChannelIds: readonly string[],
): Promise<KanbanBoard[]> {
  if (!me) return [];
  const events = await relayClient.fetchEvents({
    kinds: [KIND_KANBAN_BOARD],
    limit: BOARD_SCAN_LIMIT,
  });
  return collapseBoards(events).filter((board) =>
    boardIsAccessible(board, me, memberChannelIds),
  );
}

async function fetchBoard(
  boardId: string,
  me: string | undefined,
  memberChannelIds: readonly string[],
): Promise<KanbanBoard | null> {
  if (!me) return null;
  const events = await relayClient.fetchEvents({
    kinds: [KIND_KANBAN_BOARD],
    "#d": [boardId],
    limit: 20,
  });
  const boards = collapseBoards(events);
  return (
    boards.find((board) => boardIsAccessible(board, me, memberChannelIds)) ??
    null
  );
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

/** Every board the current user can see (own + invited + channel-shared). */
export function useBoardsQuery(
  me: string | undefined,
  memberChannelIds: readonly string[],
) {
  return useQuery({
    enabled: Boolean(me),
    queryKey: boardsQueryKey(),
    queryFn: () => fetchBoards(me, memberChannelIds),
    staleTime: 15_000,
  });
}

/** A single board head by id. */
export function useBoardQuery(
  boardId: string,
  me: string | undefined,
  memberChannelIds: readonly string[],
) {
  return useQuery({
    enabled: Boolean(me),
    queryKey: boardQueryKey(boardId),
    queryFn: () => fetchBoard(boardId, me, memberChannelIds),
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
