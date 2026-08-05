/**
 * Data layer for the Documents vault.
 *
 * Every query key is scoped by `vaultPath`, so switching vaults (or clearing
 * one) naturally invalidates without any manual cache reset. That is also why
 * none of this needs wiring into `resetCommunityState()` — the vault is global
 * and per-machine, not community-scoped.
 */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  listVaultFiles,
  readVaultFile,
  readVaultFiles,
} from "@/shared/api/vault";
import type { VaultEntry } from "@/shared/api/vaultTypes";
import { collectFilePaths } from "@/features/documents/lib/treeModel";

export function vaultTreeQueryKey(vaultPath: string | null) {
  return ["documents", "tree", vaultPath] as const;
}

export function vaultContentsQueryKey(vaultPath: string | null) {
  return ["documents", "contents", vaultPath] as const;
}

export function vaultFileQueryKey(vaultPath: string | null, path: string) {
  return ["documents", "file", vaultPath, path] as const;
}

/**
 * The whole vault tree.
 *
 * `staleTime: Infinity` because the filesystem watcher is the invalidation
 * signal — refetching on focus would just duplicate work the watcher already
 * does, and on a large vault the walk is not free.
 */
export function useVaultTreeQuery(vaultPath: string | null) {
  return useQuery({
    enabled: Boolean(vaultPath),
    queryFn: () => listVaultFiles(),
    queryKey: vaultTreeQueryKey(vaultPath),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** A single note's contents. */
export function useVaultFileQuery(
  vaultPath: string | null,
  path: string | null,
) {
  return useQuery({
    enabled: Boolean(vaultPath && path),
    // `path` is non-null whenever the query is enabled.
    queryFn: () => readVaultFile(path as string),
    queryKey: vaultFileQueryKey(vaultPath, path ?? ""),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * Every note's raw text, keyed by path — the corpus the note index and
 * backlinks are built from.
 *
 * One batched IPC call rather than Onyx's per-file fan-out.
 */
export function useVaultContentsQuery(
  vaultPath: string | null,
  tree: VaultEntry[] | undefined,
) {
  const paths = React.useMemo(
    () => (tree ? collectFilePaths(tree) : []),
    [tree],
  );

  return useQuery({
    enabled: Boolean(vaultPath) && paths.length > 0,
    queryFn: async () => {
      const results = await readVaultFiles(paths);
      const contents = new Map<string, string>();
      for (const result of results) {
        if (result.content !== null) contents.set(result.path, result.content);
      }
      return contents;
    },
    queryKey: [...vaultContentsQueryKey(vaultPath), paths.length] as const,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Invalidators for the filesystem watcher and vault mutations to call. */
export function useVaultInvalidation(vaultPath: string | null) {
  const queryClient = useQueryClient();

  const invalidateTree = React.useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: vaultTreeQueryKey(vaultPath),
    });
    void queryClient.invalidateQueries({
      queryKey: vaultContentsQueryKey(vaultPath),
    });
  }, [queryClient, vaultPath]);

  const invalidateFile = React.useCallback(
    (path: string) => {
      void queryClient.invalidateQueries({
        queryKey: vaultFileQueryKey(vaultPath, path),
      });
    },
    [queryClient, vaultPath],
  );

  return { invalidateFile, invalidateTree };
}
