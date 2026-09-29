import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useChannelsQuery } from "@/features/channels/hooks";
import { KIND_COMMUNITY_SECTIONS } from "@/shared/constants/kinds";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { canManageCommunityMembers } from "@/shared/api/relayMembers";
import type { Channel } from "@/shared/api/types";

import {
  COMMUNITY_SECTIONS_D_TAG,
  fetchCommunitySections,
  publishCommunitySections,
} from "./lib/catalog";
import { createCommunitySectionId } from "./lib/catalogParse";
import {
  COMMUNITY_SECTION_SUBS_EVENT,
  isCommunitySectionSubscribed,
  notifyCommunitySectionSubsChange,
  readCommunitySectionSubs,
  setCommunitySectionSubscribed,
  subscribedCommunitySectionIds,
  writeCommunitySectionSubs,
  type CommunitySectionSubsChange,
} from "./lib/subscriptions";
import {
  COMMUNITY_SECTIONS_ENABLED_EVENT,
  notifyCommunitySectionsEnabledChange,
  readCommunitySectionsEnabled,
  writeCommunitySectionsEnabled,
  type CommunitySectionsEnabledChange,
} from "./lib/featurePreference";
import type {
  CommunitySection,
  CommunitySectionDraft,
  CommunitySectionSubscriptionStore,
} from "./lib/types";
import { MAX_SECTION_NAME_LEN } from "./lib/types";

export const communitySectionsQueryKey = [
  "community-sections",
  "catalog",
] as const;

export function useCommunitySectionsScope() {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  return {
    pubkey: identityQuery.data?.pubkey ?? "",
    relayUrl: activeCommunity?.relayUrl ?? "",
  };
}

export function useCommunitySectionsCatalogQuery() {
  const { relayUrl } = useCommunitySectionsScope();
  return useQuery({
    queryKey: [...communitySectionsQueryKey, relayUrl],
    enabled: Boolean(relayUrl),
    queryFn: () => fetchCommunitySections(relayUrl),
    staleTime: 30_000,
  });
}

export function useCommunitySectionSubscriptions(
  pubkey: string,
  relayUrl: string,
): {
  store: CommunitySectionSubscriptionStore;
  subscribedIds: ReadonlySet<string>;
  setSubscribed: (sectionId: string, subscribed: boolean) => void;
} {
  const [store, setStore] = React.useState<CommunitySectionSubscriptionStore>(
    () => readCommunitySectionSubs(pubkey, relayUrl),
  );

  React.useEffect(() => {
    setStore(readCommunitySectionSubs(pubkey, relayUrl));
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    const onChange = (event: Event) => {
      const detail = (event as CustomEvent<CommunitySectionSubsChange>).detail;
      if (!detail) return;
      if (detail.pubkey !== pubkey || detail.relayUrl !== relayUrl) return;
      setStore(readCommunitySectionSubs(pubkey, relayUrl));
    };
    window.addEventListener(COMMUNITY_SECTION_SUBS_EVENT, onChange);
    return () =>
      window.removeEventListener(COMMUNITY_SECTION_SUBS_EVENT, onChange);
  }, [pubkey, relayUrl]);

  const setSubscribed = React.useCallback(
    (sectionId: string, subscribed: boolean) => {
      if (!pubkey || !relayUrl || !sectionId) return;
      setStore((prev) => {
        const next = setCommunitySectionSubscribed(prev, sectionId, subscribed);
        writeCommunitySectionSubs(pubkey, relayUrl, next);
        notifyCommunitySectionSubsChange({
          pubkey,
          relayUrl,
          subscribedIds: subscribedCommunitySectionIds(next),
        });
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const subscribedIds = React.useMemo(() => {
    return new Set(subscribedCommunitySectionIds(store));
  }, [store]);

  return { store, subscribedIds, setSubscribed };
}

export function useCommunitySectionsEnabled(
  pubkey: string,
  relayUrl: string,
): {
  enabled: boolean;
  setEnabled: (enabled: boolean) => void;
} {
  const [enabled, setEnabledState] = React.useState(() =>
    readCommunitySectionsEnabled(pubkey, relayUrl),
  );

  React.useEffect(() => {
    setEnabledState(readCommunitySectionsEnabled(pubkey, relayUrl));
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    const onChange = (event: Event) => {
      const detail = (event as CustomEvent<CommunitySectionsEnabledChange>)
        .detail;
      if (!detail) return;
      if (detail.pubkey !== pubkey || detail.relayUrl !== relayUrl) return;
      setEnabledState(detail.enabled);
    };
    const onStorage = (event: StorageEvent) => {
      if (!event.key) return;
      setEnabledState(readCommunitySectionsEnabled(pubkey, relayUrl));
    };
    window.addEventListener(COMMUNITY_SECTIONS_ENABLED_EVENT, onChange);
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener(COMMUNITY_SECTIONS_ENABLED_EVENT, onChange);
      window.removeEventListener("storage", onStorage);
    };
  }, [pubkey, relayUrl]);

  const setEnabled = React.useCallback(
    (next: boolean) => {
      if (!pubkey || !relayUrl) return;
      writeCommunitySectionsEnabled(pubkey, relayUrl, next);
      setEnabledState(next);
      notifyCommunitySectionsEnabledChange({
        pubkey,
        relayUrl,
        enabled: next,
      });
    },
    [pubkey, relayUrl],
  );

  return { enabled, setEnabled };
}

export function useCommunitySections() {
  const queryClient = useQueryClient();
  const { pubkey, relayUrl } = useCommunitySectionsScope();
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const canManage = canManageCommunityMembers(membershipQuery.data);
  const catalogQuery = useCommunitySectionsCatalogQuery();
  const channelsQuery = useChannelsQuery({ enabled: Boolean(relayUrl) });
  const { store, subscribedIds, setSubscribed } =
    useCommunitySectionSubscriptions(pubkey, relayUrl);
  const {
    enabled: communitySectionsEnabled,
    setEnabled: setCommunitySectionsEnabled,
  } = useCommunitySectionsEnabled(pubkey, relayUrl);

  React.useEffect(() => {
    if (!relayUrl) return;
    let disposed = false;
    let dispose: (() => void) | undefined;

    void relayClient
      .subscribeLive(
        {
          kinds: [KIND_COMMUNITY_SECTIONS],
          "#d": [COMMUNITY_SECTIONS_D_TAG],
          limit: 0,
        },
        () => {
          void queryClient.invalidateQueries({
            queryKey: [...communitySectionsQueryKey, relayUrl],
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
        console.error("Failed to subscribe to community sections", error);
      });

    const unsubReconnect = relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({
        queryKey: [...communitySectionsQueryKey, relayUrl],
      });
    });

    return () => {
      disposed = true;
      unsubReconnect();
      dispose?.();
    };
  }, [queryClient, relayUrl]);

  const sections = catalogQuery.data ?? [];

  const streamChannels = React.useMemo(() => {
    const channels = channelsQuery.data ?? [];
    return channels.filter((channel) => channel.channelType === "stream");
  }, [channelsQuery.data]);

  const saveMutation = useMutation({
    mutationFn: async (next: CommunitySection[]) => {
      if (!canManage) {
        throw new Error(
          "Only a community owner or admin can edit community sections.",
        );
      }
      await publishCommunitySections(next, relayUrl);
      return next;
    },
    onSuccess: (next) => {
      queryClient.setQueryData([...communitySectionsQueryKey, relayUrl], next);
    },
  });

  const createSection = React.useCallback(
    async (draft: CommunitySectionDraft) => {
      const name = draft.name.trim();
      if (!name || name.length > MAX_SECTION_NAME_LEN) {
        throw new Error(
          `Enter a section name (max ${MAX_SECTION_NAME_LEN} characters).`,
        );
      }
      const section: CommunitySection = {
        id: createCommunitySectionId(),
        name,
        ...(draft.icon?.trim() ? { icon: draft.icon.trim() } : {}),
        order: sections.length,
        channelIds: [...draft.channelIds],
      };
      await saveMutation.mutateAsync([...sections, section]);
      return section;
    },
    [saveMutation, sections],
  );

  const updateSection = React.useCallback(
    async (sectionId: string, draft: CommunitySectionDraft) => {
      const name = draft.name.trim();
      if (!name || name.length > MAX_SECTION_NAME_LEN) {
        throw new Error(
          `Enter a section name (max ${MAX_SECTION_NAME_LEN} characters).`,
        );
      }
      const next = sections.map((section) =>
        section.id === sectionId
          ? {
              ...section,
              name,
              ...(draft.icon?.trim()
                ? { icon: draft.icon.trim() }
                : { icon: undefined }),
              channelIds: [...draft.channelIds],
            }
          : section,
      );
      await saveMutation.mutateAsync(next);
    },
    [saveMutation, sections],
  );

  const deleteSection = React.useCallback(
    async (sectionId: string) => {
      const next = sections
        .filter((section) => section.id !== sectionId)
        .map((section, index) => ({ ...section, order: index }));
      await saveMutation.mutateAsync(next);
      if (isCommunitySectionSubscribed(store, sectionId)) {
        setSubscribed(sectionId, false);
      }
    },
    [saveMutation, sections, setSubscribed, store],
  );

  const subscribedSections = React.useMemo(() => {
    return sections.filter((section) => subscribedIds.has(section.id));
  }, [sections, subscribedIds]);

  const channelsById = React.useMemo(() => {
    const map = new Map<string, Channel>();
    for (const channel of streamChannels) {
      map.set(channel.id, channel);
    }
    return map;
  }, [streamChannels]);

  const navSubscribedSections = React.useMemo(() => {
    return communitySectionsEnabled ? subscribedSections : [];
  }, [communitySectionsEnabled, subscribedSections]);

  const lockedChannelIds = React.useMemo(() => {
    if (!communitySectionsEnabled) return new Set<string>();
    const ids = new Set<string>();
    for (const section of subscribedSections) {
      for (const channelId of section.channelIds) {
        ids.add(channelId);
      }
    }
    return ids;
  }, [communitySectionsEnabled, subscribedSections]);

  return {
    sections,
    subscribedSections,
    /** Subscribed sections to render in the left nav (empty when master toggle is off). */
    navSubscribedSections,
    subscribedIds,
    streamChannels,
    channelsById,
    canManage,
    communitySectionsEnabled,
    setCommunitySectionsEnabled,
    lockedChannelIds,
    isLoading: catalogQuery.isLoading,
    isSaving: saveMutation.isPending,
    createSection,
    updateSection,
    deleteSection,
    setSubscribed,
    isSubscribed: (sectionId: string) => subscribedIds.has(sectionId),
  };
}
