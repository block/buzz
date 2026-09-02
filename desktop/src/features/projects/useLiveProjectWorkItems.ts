import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import {
  createTrailingDebounce,
  type TrailingDebounce,
} from "@/shared/lib/trailingDebounce";
import type { Project } from "./hooks";

const WORK_ITEM_KINDS = [
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_ISSUE,
  KIND_GIT_STATUS_OPEN,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_TEXT_NOTE,
];

const RETRY_BASE_MS = 1_000;
const RETRY_MAX_MS = 30_000;
// Work-item traffic arrives in bursts (a PR push emits update + status +
// comments); coalesce into one trailing refetch.
const INVALIDATE_DEBOUNCE_MS = 500;
// Overlap the live filter with already-fetched history so an event landing
// between the last query fetch and the subscription start is not lost.
const SUBSCRIBE_OVERLAP_SECONDS = 5;

/**
 * Keeps the project's task and review queries fresh while the project home
 * is open: one live relay subscription over the project's own repo addresses
 * (never widget-supplied scope) that debounce-invalidates the React Query
 * caches feeding the canvas broker and the Tasks/Reviews tabs.
 */
export function useLiveProjectWorkItems(project: Project | null | undefined) {
  const queryClient = useQueryClient();
  const repositories = project?.repositories;
  const repoAddressesKey = React.useMemo(
    () =>
      [...new Set((repositories ?? []).map((repo) => repo.repoAddress))]
        .sort()
        .join(","),
    [repositories],
  );
  const repositoryIdsRef = React.useRef<string[]>([]);
  repositoryIdsRef.current = (repositories ?? []).map((repo) => repo.id);

  const invalidateRef = React.useRef<TrailingDebounce | null>(null);
  if (invalidateRef.current === null) {
    invalidateRef.current = createTrailingDebounce(() => {
      for (const repositoryId of repositoryIdsRef.current) {
        void queryClient.invalidateQueries({
          queryKey: ["project", repositoryId, "issues"],
        });
        void queryClient.invalidateQueries({
          queryKey: ["project", repositoryId, "pull-requests"],
        });
      }
      void queryClient.invalidateQueries({
        queryKey: ["projects", "work-items"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["projects", "activity-summaries"],
      });
    }, INVALIDATE_DEBOUNCE_MS);
  }

  React.useEffect(() => {
    return relayClient.subscribeToReconnects(() => {
      invalidateRef.current?.trigger();
    });
  }, []);

  React.useEffect(() => {
    const repoAddresses = repoAddressesKey ? repoAddressesKey.split(",") : [];
    if (repoAddresses.length === 0) return;

    let cancelled = false;
    let retryTimeout: number | undefined;
    let retryAttempt = 0;
    let dispose: (() => Promise<void>) | null = null;

    const subscribe = async () => {
      try {
        const nextDispose = await relayClient.subscribeLive(
          {
            kinds: WORK_ITEM_KINDS,
            "#a": repoAddresses,
            limit: 1_000,
            since: Math.floor(Date.now() / 1_000) - SUBSCRIBE_OVERLAP_SECONDS,
          },
          () => invalidateRef.current?.trigger(),
        );
        if (cancelled) {
          void nextDispose().catch(() => {});
          return;
        }
        dispose = nextDispose;
        retryAttempt = 0;
        // Close the fetch/subscription gap: anything published while we were
        // not yet subscribed is picked up by an immediate refetch.
        invalidateRef.current?.trigger();
      } catch {
        if (cancelled) return;
        const delayMs = Math.min(
          RETRY_BASE_MS * 2 ** retryAttempt,
          RETRY_MAX_MS,
        );
        retryAttempt += 1;
        retryTimeout = window.setTimeout(() => {
          retryTimeout = undefined;
          void subscribe();
        }, delayMs);
      }
    };

    void subscribe();

    return () => {
      cancelled = true;
      if (retryTimeout !== undefined) window.clearTimeout(retryTimeout);
      if (dispose) void dispose().catch(() => {});
      dispose = null;
    };
  }, [repoAddressesKey]);

  React.useEffect(
    () => () => {
      invalidateRef.current?.cancel();
    },
    [],
  );
}
