import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { useMyRelayMembershipQuery } from "@/features/relay-members/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
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

/** `undefined` while the snapshot loads so callers can defer the flair. */
export function useIsIdentityArchived(pubkey: string): boolean | undefined {
  const query = useArchivedIdentitiesQuery();
  if (!query.data) return undefined;
  const lower = pubkey.toLowerCase();
  return query.data.archived.includes(lower);
}

/**
 * Predicate for hiding archived identities from forward-looking discovery
 * surfaces (autocomplete, DM picker, member-adder, search, panel-fold).
 * Fail-open: returns `false` while the snapshot loads so a cold start can't
 * briefly hide everyone.
 *
 * Self-exempt by construction: the current user is never folded from their own
 * client, even when archived on the relay. NIP-IA §Self Requests makes archival
 * deliberately non-silent — the anti-shadowban property requires the archived
 * user to see they're archived and self-unarchive. Folding self would build the
 * exact shadowban the NIP prevents, so the exemption lives here in the
 * predicate where no caller can forget it.
 */
export function useIsArchivedPredicate(): (pubkey: string) => boolean {
  const query = useArchivedIdentitiesQuery();
  const identityQuery = useIdentityQuery();
  const selfPubkey = identityQuery.data?.pubkey;
  return React.useMemo(() => {
    const self = selfPubkey?.toLowerCase() ?? null;
    const set = new Set(
      (query.data?.archived ?? []).map((p) => p.toLowerCase()),
    );
    return (pubkey: string) => {
      const lower = pubkey.toLowerCase();
      return lower !== self && set.has(lower);
    };
  }, [query.data, selfPubkey]);
}

/** Gates the owner-path archive button via the target's live `kind:0`. */
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

/** Everything the profile panel needs to gate + drive NIP-IA archival. */
export type IdentityArchiveActions = {
  /** Render guard only — the relay re-verifies authority on submit. */
  canArchive: boolean;
  /** `undefined` while the snapshot loads — defer flair + Manage until known. */
  isArchived: boolean | undefined;
  isPending: boolean;
  archive: () => void;
  unarchive: () => void;
};

/**
 * Self-contained NIP-IA archive controller for a single `pubkey`. Composes the
 * gate queries, owns both mutations, and exposes archive/unarchive with toasts.
 *
 * Safe to call from multiple components on the same `pubkey`: React Query
 * dedupes the underlying subscriptions by queryKey, so a second hook call costs
 * a render, not a second network round-trip.
 */
export function useIdentityArchive(pubkey: string): IdentityArchiveActions {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;

  const pubkeyLower = pubkey.toLowerCase();
  const isSelf =
    currentPubkey !== undefined && pubkeyLower === currentPubkey.toLowerCase();

  const myMembershipQuery = useMyRelayMembershipQuery();
  // Skip the kind:0 lookup when viewing yourself — the OA gate is for
  // archiving *other* identities you own. Also defer until our own identity
  // resolves so we never fire the lookup against an unknown viewer.
  const oaOwnerQuery = useOaOwnerQuery(
    pubkey,
    currentPubkey !== undefined && !isSelf,
  );

  const isArchived = useIsIdentityArchived(pubkey);

  const archiveMutation = useArchiveIdentityMutation();
  const unarchiveMutation = useUnarchiveIdentityMutation();

  const myRole = myMembershipQuery.data?.role;
  const isRelayAdminOrOwner = myRole === "owner" || myRole === "admin";
  const isOaOwnerOfViewee = oaOwnerQuery.data?.isMe === true;
  const canArchive = isSelf || isRelayAdminOrOwner || isOaOwnerOfViewee;

  const archive = React.useCallback(() => {
    archiveMutation.mutate(
      { targetPubkey: pubkey },
      {
        onSuccess: () => toast.success("Archived on this relay"),
        onError: (error) =>
          toast.error(
            `Archive failed: ${error instanceof Error ? error.message : String(error)}`,
          ),
      },
    );
  }, [archiveMutation, pubkey]);

  const unarchive = React.useCallback(() => {
    unarchiveMutation.mutate(
      { targetPubkey: pubkey },
      {
        onSuccess: () => toast.success("Unarchived on this relay"),
        onError: (error) =>
          toast.error(
            `Unarchive failed: ${error instanceof Error ? error.message : String(error)}`,
          ),
      },
    );
  }, [pubkey, unarchiveMutation]);

  return {
    canArchive,
    isArchived,
    isPending: archiveMutation.isPending || unarchiveMutation.isPending,
    archive,
    unarchive,
  };
}
