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
 * Predicate for hiding archived identities from forward-looking discovery
 * surfaces (mention autocomplete, DM picker, member-adder, search,
 * panel-fold). Distinct from `useIsIdentityArchived` because callers here
 * need a synchronous boolean: while the `kind:13535` snapshot is loading the
 * predicate returns `false` (no-op — show everyone), never `true` — fail-open
 * so a cold-start can't briefly hide everyone.
 *
 * Self-exempt by construction: the current user is **never** filtered or
 * folded from their own client, even when archived on the relay. NIP-IA §Self
 * Requests makes archival deliberately non-silent — the anti-shadowban
 * property requires the archived user to see they're archived and be able to
 * self-unarchive. The profile pane's "Archived" flair is the honest
 * disclosure; removing self from member lists / autocomplete / search would
 * build the exact shadowban the NIP is designed to prevent. Self-exemption
 * lives here, in the predicate, so no caller can forget it.
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

/** Everything the profile panel needs to gate + drive NIP-IA archival. */
export type IdentityArchiveActions = {
  /**
   * UX gate. `true` when ANY auth path will be accepted by the relay: self
   * (acting on own pubkey), relay admin/owner, or verified NIP-OA owner of the
   * viewee. The relay re-verifies on submit — this is purely a render guard.
   */
  canArchive: boolean;
  /**
   * `true` iff the target is in the relay's latest `kind:13535` snapshot.
   * `undefined` while the snapshot loads so callers can defer the flair /
   * Manage section until authority + state are both known.
   */
  isArchived: boolean | undefined;
  /** Either mutation in flight — drives the disabled + "Archiving…" states. */
  isPending: boolean;
  /** Submit a `kind:9035` archive request for `pubkey` (toasts on result). */
  archive: () => void;
  /** Submit a `kind:9036` unarchive request for `pubkey` (toasts on result). */
  unarchive: () => void;
};

/**
 * Self-contained NIP-IA archive controller for a single `pubkey`. Composes the
 * three gate queries, owns both mutations, and exposes the archive/unarchive
 * callbacks with toasts — collapsing what used to be six props drilled through
 * the profile panel into one hook call.
 *
 * Safe to call from multiple components on the same `pubkey`: React Query
 * dedupes the underlying subscriptions by queryKey, so the only cost is a
 * second hook invocation, not a second network round-trip.
 *
 * Gate composition is verbatim from the old `UserProfilePanel`:
 * `canArchive = isSelf || isRelayAdminOrOwner || isOaOwnerOfViewee`.
 */
export function useIdentityArchive(pubkey: string): IdentityArchiveActions {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;

  const pubkeyLower = pubkey.toLowerCase();
  const isSelf =
    currentPubkey !== undefined &&
    pubkeyLower === currentPubkey.toLowerCase();

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
