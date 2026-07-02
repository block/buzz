import { useQueries, useQuery } from "@tanstack/react-query";

import { fetchWorkspaceIcon } from "@/shared/api/workspaceProfile";

import type { Workspace } from "./types";
import {
  loadCachedWorkspaceIcon,
  saveCachedWorkspaceIcon,
} from "./workspaceIconCache";

export const workspaceIconQueryKey = (relayUrl: string) =>
  ["workspaceIcon", relayUrl] as const;

const ICON_STALE_MS = 5 * 60_000;

async function fetchIconForWorkspace(
  workspace: Workspace,
): Promise<string | null> {
  const icon = await fetchWorkspaceIcon(workspace.relayUrl);
  saveCachedWorkspaceIcon(workspace.relayUrl, icon);
  return icon;
}

function iconQueryOptions(workspace: Workspace) {
  return {
    queryKey: workspaceIconQueryKey(workspace.relayUrl),
    queryFn: () => fetchIconForWorkspace(workspace),
    // Cached icon renders immediately; the fetch still runs and replaces it.
    placeholderData: loadCachedWorkspaceIcon(workspace.relayUrl),
    staleTime: ICON_STALE_MS,
    retry: 1,
  };
}

/**
 * Workspace icons for the rail, keyed by workspace id. Each icon is read
 * from its relay's NIP-11 document over plain HTTP — active and inactive
 * workspaces alike. Falls back to the localStorage cache (then null →
 * initials) when a relay is unreachable.
 */
export function useWorkspaceIcons(
  workspaces: Workspace[],
): Record<string, string | null> {
  const results = useQueries({
    queries: workspaces.map((workspace) => iconQueryOptions(workspace)),
  });

  const icons: Record<string, string | null> = {};
  workspaces.forEach((workspace, index) => {
    icons[workspace.id] =
      results[index]?.data ?? loadCachedWorkspaceIcon(workspace.relayUrl);
  });
  return icons;
}

/** Icon of the ACTIVE workspace, for settings preview. */
export function useActiveWorkspaceIcon(relayUrl: string | undefined) {
  return useQuery({
    queryKey: workspaceIconQueryKey(relayUrl ?? ""),
    queryFn: async () => {
      const icon = await fetchWorkspaceIcon(relayUrl ?? "");
      if (relayUrl) saveCachedWorkspaceIcon(relayUrl, icon);
      return icon;
    },
    enabled: relayUrl !== undefined,
    staleTime: ICON_STALE_MS,
  });
}
