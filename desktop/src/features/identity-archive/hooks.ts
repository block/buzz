import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  archiveIdentity,
  listArchivedIdentities,
  resolveOaOwner,
  unarchiveIdentity,
  type ArchivedIdentitiesSnapshot,
  type IdentityArchiveRequest,
  type IdentityUnarchiveRequest,
} from "@/shared/api/tauriIdentityArchive";

export const archivedIdentitiesQueryKey = ["archivedIdentities"] as const;

/** Cache the relay's `kind:13535` snapshot. Drives the "Archived" flair. */
export function useArchivedIdentitiesQuery(enabled = true) {
  return useQuery<ArchivedIdentitiesSnapshot>({
    enabled,
    queryKey: archivedIdentitiesQueryKey,
    queryFn: listArchivedIdentities,
    staleTime: 30_000,
  });
}

/**
 * `true` iff `pubkey` appears in the relay's latest archive snapshot.
 * Returns `undefined` while the snapshot is loading so callers can hide the
 * flair until we know.
 */
export function useIsIdentityArchived(pubkey: string): boolean | undefined {
  const query = useArchivedIdentitiesQuery();
  if (!query.data) return undefined;
  const lower = pubkey.toLowerCase();
  return query.data.archived.includes(lower);
}

/**
 * Resolve the NIP-OA owner of a target via its live `kind:0`. Gates the
 * owner-path archive button.
 */
export function useOaOwnerQuery(pubkey: string, enabled = true) {
  return useQuery({
    enabled,
    queryKey: ["oaOwner", pubkey.toLowerCase()] as const,
    queryFn: () => resolveOaOwner(pubkey),
    staleTime: 60_000,
  });
}

export function useArchiveIdentityMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (req: IdentityArchiveRequest) => archiveIdentity(req),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: archivedIdentitiesQueryKey,
      });
    },
  });
}

export function useUnarchiveIdentityMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (req: IdentityUnarchiveRequest) => unarchiveIdentity(req),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: archivedIdentitiesQueryKey,
      });
    },
  });
}
