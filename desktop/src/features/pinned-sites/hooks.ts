import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { KIND_COMMUNITY_PINNED_SITES } from "@/shared/constants/kinds";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { canManageCommunityMembers } from "@/shared/api/relayMembers";

import {
  COMMUNITY_PINNED_SITES_D_TAG,
  fetchCommunityPinnedSitesWithRunbooks,
  publishCommunityPinnedSites,
} from "./lib/communityPins";
import { pinRunbookRef } from "@/features/site-runbook/lib/keys";
import { mergeCommunityRunbook } from "@/features/site-runbook/lib/mutations";
import {
  clearSiteRunbook,
  configureSiteRunbooksScope,
  getSiteRunbook,
  setSiteRunbook,
} from "@/features/site-runbook/lib/store";
import { mergePinnedSites } from "./lib/mergePins";
import { closePinWebview } from "./lib/pinWebview";
import {
  loadPersonalPinnedSites,
  removePersonalPin,
  savePersonalPinnedSites,
  upsertPersonalPin,
  type PersonalPinnedSitesBlob,
} from "./lib/personalPins";
import type { PinnedSite, PinnedSiteDraft } from "./lib/types";
import { normalizePinnedSiteName, normalizePinnedSiteUrl } from "./lib/url";

export const personalPinnedSitesQueryKey = [
  "pinned-sites",
  "personal",
] as const;
export const communityPinnedSitesQueryKey = [
  "pinned-sites",
  "community",
] as const;

function createPinId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `pin-${Date.now().toString(36)}`;
}

function draftToPin(draft: PinnedSiteDraft, id = createPinId()): PinnedSite {
  const name = normalizePinnedSiteName(draft.name);
  const url = normalizePinnedSiteUrl(draft.url);
  if (!name || !url) {
    throw new Error("Enter a name and an https URL.");
  }
  return {
    id,
    name,
    url,
    icon: draft.icon,
    pollForChanges: draft.pollForChanges,
    openMatchingLinks: draft.openMatchingLinks !== false,
    scope: draft.community ? "community" : "personal",
  };
}

export function usePinnedSitesScope() {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  return {
    pubkey: identityQuery.data?.pubkey ?? "",
    relayUrl: activeCommunity?.relayUrl ?? "",
  };
}

export function usePersonalPinnedSitesQuery() {
  const { pubkey, relayUrl } = usePinnedSitesScope();
  return useQuery({
    queryKey: [...personalPinnedSitesQueryKey, pubkey, relayUrl],
    enabled: Boolean(pubkey && relayUrl),
    queryFn: (): PersonalPinnedSitesBlob => {
      const blob = loadPersonalPinnedSites(pubkey, relayUrl);
      savePersonalPinnedSites(pubkey, relayUrl, blob);
      return blob;
    },
  });
}

export function useCommunityPinnedSitesQuery() {
  const { pubkey, relayUrl } = usePinnedSitesScope();
  return useQuery({
    queryKey: communityPinnedSitesQueryKey,
    queryFn: async () => {
      if (pubkey && relayUrl) {
        configureSiteRunbooksScope(pubkey, relayUrl);
      }
      const rows = await fetchCommunityPinnedSitesWithRunbooks();
      for (const row of rows) {
        if (!row.runbook) continue;
        const ref = pinRunbookRef(row.pin.id);
        const merged = mergeCommunityRunbook(getSiteRunbook(ref), row.runbook);
        setSiteRunbook(ref, merged);
      }
      return rows.map((row) => row.pin);
    },
    staleTime: 30_000,
  });
}

export function useCommunityPinnedSitesLiveUpdates(): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    let disposed = false;
    let dispose: (() => void) | undefined;

    void relayClient
      .subscribeLive(
        {
          kinds: [KIND_COMMUNITY_PINNED_SITES],
          "#d": [COMMUNITY_PINNED_SITES_D_TAG],
          limit: 0,
        },
        () => {
          void queryClient.invalidateQueries({
            queryKey: communityPinnedSitesQueryKey,
          });
        },
      )
      .then((unsubscribe) => {
        if (disposed) {
          void unsubscribe();
        } else {
          dispose = () => {
            void unsubscribe();
          };
        }
      })
      .catch((error) => {
        console.error("Failed to subscribe to community pinned sites", error);
      });

    const unsubReconnect = relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({
        queryKey: communityPinnedSitesQueryKey,
      });
    });

    return () => {
      disposed = true;
      unsubReconnect();
      dispose?.();
    };
  }, [queryClient]);
}

export function usePinnedSites() {
  const queryClient = useQueryClient();
  const { pubkey, relayUrl } = usePinnedSitesScope();
  const personalQuery = usePersonalPinnedSitesQuery();
  const communityQuery = useCommunityPinnedSitesQuery();
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const canShareCommunity = canManageCommunityMembers(membershipQuery.data);

  const pins = React.useMemo(
    () =>
      mergePinnedSites(
        personalQuery.data?.pins ?? [],
        communityQuery.data ?? [],
      ),
    [communityQuery.data, personalQuery.data?.pins],
  );

  const persistPersonal = React.useCallback(
    (blob: PersonalPinnedSitesBlob) => {
      if (!pubkey || !relayUrl) return;
      savePersonalPinnedSites(pubkey, relayUrl, blob);
      void queryClient.invalidateQueries({
        queryKey: personalPinnedSitesQueryKey,
      });
    },
    [pubkey, queryClient, relayUrl],
  );

  const persistCommunity = React.useCallback(
    async (next: PinnedSite[]) => {
      if (pubkey && relayUrl) {
        configureSiteRunbooksScope(pubkey, relayUrl);
      }
      const runbooks = new Map(
        next.map((pin) => [pin.id, getSiteRunbook(pinRunbookRef(pin.id))]),
      );
      await publishCommunityPinnedSites(next, runbooks);
      void queryClient.invalidateQueries({
        queryKey: communityPinnedSitesQueryKey,
      });
    },
    [pubkey, queryClient, relayUrl],
  );

  const saveMutation = useMutation({
    mutationFn: async (input: { id?: string; draft: PinnedSiteDraft }) => {
      const pin = draftToPin(input.draft, input.id);
      const personal = personalQuery.data;
      if (!personal || !communityQuery.isFetched) {
        throw new Error("Pinned sites are still loading.");
      }
      const community = [...(communityQuery.data ?? [])];

      if (pin.scope === "community") {
        if (!canShareCommunity) {
          throw new Error("Only a community owner or admin can share a pin.");
        }
        const nextCommunity = [
          ...community.filter((entry) => entry.id !== pin.id),
          pin,
        ];
        persistPersonal(removePersonalPin(personal, pin.id));
        await persistCommunity(nextCommunity);
        return pin;
      }

      if (input.id && community.some((entry) => entry.id === input.id)) {
        if (!canShareCommunity) {
          throw new Error("Only a community owner or admin can edit this pin.");
        }
        await persistCommunity(
          community.filter((entry) => entry.id !== pin.id),
        );
      }
      persistPersonal(upsertPersonalPin(personal, pin));
      return pin;
    },
  });

  const deleteMutation = useMutation({
    mutationFn: async (pin: PinnedSite) => {
      if (pin.scope === "community") {
        if (!canShareCommunity) {
          throw new Error(
            "Only a community owner or admin can delete a community pin.",
          );
        }
        if (!communityQuery.isFetched) {
          throw new Error("Pinned sites are still loading.");
        }
        await persistCommunity(
          (communityQuery.data ?? []).filter((entry) => entry.id !== pin.id),
        );
      } else {
        const personal = personalQuery.data;
        if (!personal) {
          throw new Error("Pinned sites are still loading.");
        }
        persistPersonal(removePersonalPin(personal, pin.id));
      }
      clearSiteRunbook(pinRunbookRef(pin.id));
      await closePinWebview(pin.id);
    },
  });

  return {
    pins,
    canShareCommunity,
    isLoading: personalQuery.isLoading || communityQuery.isLoading,
    savePin: saveMutation.mutateAsync,
    deletePin: deleteMutation.mutateAsync,
    isSaving: saveMutation.isPending,
  };
}

export function usePinnedSite(pinId: string | undefined): PinnedSite | null {
  const { pins } = usePinnedSites();
  if (!pinId) return null;
  return pins.find((pin) => pin.id === pinId) ?? null;
}
