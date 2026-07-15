import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import {
  fetchInstalledPackCoordinates,
  fetchAllStickerPacks,
  fetchOwnStickerPacks,
  fetchStickerCatalog,
  setStickerCatalogApproval,
  setStickerPackInstalled,
  type StickerPack,
} from "@/shared/api/stickers";
import {
  KIND_STICKER_CATALOG,
  KIND_STICKER_PACK,
  KIND_USER_STICKER_PACKS,
} from "@/shared/constants/kinds";

export const stickerCatalogQueryKey = ["sticker-catalog"] as const;
export const installedStickerPacksQueryKey = [
  "sticker-packs-installed",
] as const;
export const ownStickerPacksQueryKey = ["sticker-packs-own"] as const;
export const allStickerPacksQueryKey = ["sticker-packs-all"] as const;

export function useStickerCatalogQuery() {
  return useQuery({
    queryKey: stickerCatalogQueryKey,
    queryFn: fetchStickerCatalog,
    staleTime: 60_000,
    refetchInterval: 120_000,
  });
}

export function useInstalledStickerCoordinatesQuery() {
  return useQuery({
    queryKey: installedStickerPacksQueryKey,
    queryFn: fetchInstalledPackCoordinates,
    staleTime: 60_000,
  });
}

export function useOwnStickerPacksQuery() {
  return useQuery({
    queryKey: ownStickerPacksQueryKey,
    queryFn: fetchOwnStickerPacks,
    staleTime: 60_000,
  });
}

export function useAllStickerPacksQuery(enabled = true) {
  return useQuery({
    queryKey: allStickerPacksQueryKey,
    queryFn: fetchAllStickerPacks,
    staleTime: 60_000,
    enabled,
  });
}

export function useInstalledStickerPacks(): StickerPack[] {
  const catalog = useStickerCatalogQuery().data ?? [];
  const installed = useInstalledStickerCoordinatesQuery().data ?? [];
  const packsByCoordinate = React.useMemo(
    () => new Map(catalog.map((pack) => [pack.coordinate, pack])),
    [catalog],
  );
  return React.useMemo(
    () =>
      installed.flatMap((coordinate) => {
        const pack = packsByCoordinate.get(coordinate);
        return pack ? [pack] : [];
      }),
    [installed, packsByCoordinate],
  );
}

export function useSetStickerPackInstalledMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      coordinate,
      installed,
    }: {
      coordinate: string;
      installed: boolean;
    }) => setStickerPackInstalled(coordinate, installed),
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: installedStickerPacksQueryKey,
      }),
  });
}

export function useSetStickerCatalogApprovalMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      coordinate,
      eventId,
      approved,
    }: {
      coordinate: string;
      eventId: string;
      approved: boolean;
    }) => setStickerCatalogApproval(coordinate, eventId, approved),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: stickerCatalogQueryKey }),
  });
}

/** Keep catalog and installed-list queries coherent across live events/reconnects. */
export function useStickerLiveUpdates(): void {
  const queryClient = useQueryClient();
  React.useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];
    void Promise.all([
      relayClient.subscribeLive(
        { kinds: [KIND_STICKER_CATALOG, KIND_STICKER_PACK], limit: 0 },
        () => {
          void queryClient.invalidateQueries({
            queryKey: stickerCatalogQueryKey,
          });
          void queryClient.invalidateQueries({
            queryKey: ownStickerPacksQueryKey,
          });
          void queryClient.invalidateQueries({
            queryKey: allStickerPacksQueryKey,
          });
        },
      ),
      relayClient.subscribeLive(
        { kinds: [KIND_USER_STICKER_PACKS], limit: 0 },
        () => {
          void queryClient.invalidateQueries({
            queryKey: installedStickerPacksQueryKey,
          });
        },
      ),
    ])
      .then((subscriptions) => {
        if (disposed) {
          for (const unsubscribe of subscriptions) void unsubscribe();
        } else {
          for (const unsubscribe of subscriptions)
            cleanups.push(() => void unsubscribe());
        }
      })
      .catch(() => {
        // Polling remains the backstop if a live subscription cannot be opened.
      });
    const reconnect = relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({ queryKey: stickerCatalogQueryKey });
      void queryClient.invalidateQueries({
        queryKey: installedStickerPacksQueryKey,
      });
    });
    return () => {
      disposed = true;
      reconnect();
      for (const cleanup of cleanups) cleanup();
    };
  }, [queryClient]);
}
