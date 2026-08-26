import * as React from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import {
  addChannelMembers,
  archiveChannel,
  createChannel,
  deleteChannel,
  getCanvas,
  getChannelDetails,
  getChannelMembers,
  getChannels,
  hideDm,
  joinChannel,
  leaveChannel,
  openDm,
  invokeTauri,
  removeChannelMember,
  setCanvas,
  setChannelPurpose,
  setChannelTopic,
  unarchiveChannel,
  updateChannel,
} from "@/shared/api/tauri";
import type {
  AddChannelMembersInput,
  Channel,
  ChannelDetail,
  CreateChannelInput,
  SetChannelPurposeInput,
  SetChannelTopicInput,
  UpdateChannelInput,
} from "@/shared/api/types";
import type {
  GetChannelsPayload,
  OpenDmInput,
} from "@/shared/api/tauriChannels";
import { mergeConcurrentChannelRecency } from "@/features/channels/lib/channelRecencyMerge";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  inspectChannelSnapshot,
  type ChannelSnapshot,
  writeChannelSnapshot,
} from "@/features/channels/channelSnapshot";
import {
  CHANNEL_MEMBERS_STALE_TIME_MS,
  channelMembersQueryKey,
} from "@/features/channels/rosterFreshness";

export const channelsQueryKey = ["channels"] as const;
/** Keeps focused polling at the established one-minute cadence. */
export const CHANNELS_REFETCH_INTERVAL_MS = 60_000;
/** Suppresses the expensive focus refetch until the channel list is old. */
export const CHANNELS_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy for the channels query; consumed by focusRefetchPolicy.test.mjs. */
export const channelsFocusRefetchPolicy = {
  staleTime: CHANNELS_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: false,
} as const;
/**
 * Authoritative server list/hash pair. Presentation mutations may patch
 * `channelsQueryKey`, but may never change or be persisted with this hash.
 */
const channelsSnapshotPairKey = ["channels", "_snapshot-pair"] as const;
const channelDetailQueryKey = (channelId: string) =>
  ["channels", channelId, "detail"] as const;
const channelTypeOrder = {
  stream: 0,
  forum: 1,
  dm: 2,
} as const;

export function sortChannels(channels: Channel[]) {
  const uniqueChannels = new Map<string, Channel>();

  for (const channel of channels) {
    uniqueChannels.set(channel.id, channel);
  }

  return [...uniqueChannels.values()].sort((left, right) => {
    const typeOrder =
      channelTypeOrder[left.channelType] - channelTypeOrder[right.channelType];

    if (typeOrder !== 0) {
      return typeOrder;
    }

    return left.name.localeCompare(right.name);
  });
}

export const CHANNELS_SNAPSHOT_DIAGNOSTIC_MARK =
  "buzz:sidebar:snapshot-diagnostic";
export const CHANNELS_FULL_SIDEBAR_PAINT_MARK =
  "buzz:sidebar:full-list-painted";
export const CHANNELS_BOOT_TO_FULL_SIDEBAR_MEASURE =
  "buzz:sidebar:boot-to-full-list-painted";

const markedSnapshotKeys = new Set<string>();
const measuredSidebarKeys = new Set<string>();
const scheduledSidebarKeys = new Set<string>();

function sidebarMeasurementKey(relayUrl: string, ownerPubkey: string): string {
  return `${relayUrl}\u0000${ownerPubkey.toLowerCase()}`;
}

function markSnapshotDiagnostic(
  relayUrl: string,
  ownerPubkey: string,
  diagnostics: ReturnType<typeof inspectChannelSnapshot>["diagnostics"],
): void {
  if (typeof performance === "undefined") return;
  const key = sidebarMeasurementKey(relayUrl, ownerPubkey);
  if (markedSnapshotKeys.has(key)) return;
  markedSnapshotKeys.add(key);
  performance.mark(CHANNELS_SNAPSHOT_DIAGNOSTIC_MARK, {
    detail: { ...diagnostics, relayUrl },
  });
  console.info("[sidebar-perf] snapshot", { ...diagnostics, relayUrl });
}

function measureFullSidebarPaint(
  relayUrl: string,
  ownerPubkey: string,
  channelCount: number,
): void {
  if (typeof performance === "undefined") return;
  const key = sidebarMeasurementKey(relayUrl, ownerPubkey);
  if (measuredSidebarKeys.has(key) || scheduledSidebarKeys.has(key)) return;
  scheduledSidebarKeys.add(key);

  // The channels have committed to the shared query cache; two animation frames
  // put the mark after React's sidebar DOM commit and the browser's next paint.
  window.requestAnimationFrame(() => {
    window.requestAnimationFrame(() => {
      scheduledSidebarKeys.delete(key);
      if (measuredSidebarKeys.has(key)) return;
      measuredSidebarKeys.add(key);
      performance.mark(CHANNELS_FULL_SIDEBAR_PAINT_MARK, {
        detail: { channelCount, relayUrl },
      });
      performance.measure(CHANNELS_BOOT_TO_FULL_SIDEBAR_MEASURE, {
        detail: { channelCount, relayUrl },
        duration: performance.now(),
        start: 0,
      });
      const measure = performance
        .getEntriesByName(CHANNELS_BOOT_TO_FULL_SIDEBAR_MEASURE)
        .at(-1);
      console.info("[sidebar-perf] full list painted", {
        channelCount,
        durationMs: measure?.duration,
        relayUrl,
      });
    });
  });
}

export type CachedChannelMember = {
  membershipAdded: boolean;
  name: string;
  pubkey: string;
};

/**
 * Records a successful membership mutation in the shared channel list before
 * its read-after-write refetch completes. DM participant sets are immutable,
 * so adding a member there creates a separate conversation and must never
 * decorate the source channel optimistically. Exported for focused cache race
 * regression coverage.
 */
export function upsertCachedChannelMember(
  current: Channel[] | undefined,
  channelId: string,
  member: CachedChannelMember,
): Channel[] | undefined {
  if (!current) {
    return current;
  }

  const normalizedPubkey = member.pubkey.toLowerCase();
  return sortChannels(
    current.map((channel) => {
      if (channel.id !== channelId) {
        return channel;
      }

      if (channel.channelType === "dm") {
        return channel;
      }

      const hasMember = channel.memberPubkeys.some(
        (pubkey) => pubkey.toLowerCase() === normalizedPubkey,
      );
      const memberPubkeys = hasMember
        ? channel.memberPubkeys
        : [...channel.memberPubkeys, member.pubkey];
      return {
        ...channel,
        memberCount: Math.max(
          memberPubkeys.length,
          channel.memberCount + (member.membershipAdded && !hasMember ? 1 : 0),
        ),
        memberPubkeys,
      };
    }),
  );
}

/**
 * Adds or replaces a relay-returned channel in a possibly stale channel list.
 * Exported for focused cache race regression coverage.
 */
export function upsertCachedChannel(
  current: Channel[] | undefined,
  channel: Channel,
): Channel[] {
  return sortChannels([
    ...(current ?? []).filter((candidate) => candidate.id !== channel.id),
    channel,
  ]);
}

/**
 * Reconciles a relay-returned channel after a list refresh. When the refresh
 * already contains the immutable DM, its current metadata wins over the older
 * snapshot used to open the route. Otherwise the opened channel repairs the
 * route after a read-after-write-lagged list response.
 */
export function reconcileRefreshedCachedChannel(
  refreshed: Channel[] | undefined,
  channel: Channel,
): Channel[] {
  const refreshedChannel = refreshed?.find(
    (candidate) => candidate.id === channel.id,
  );
  return upsertCachedChannel(refreshed, refreshedChannel ?? channel);
}

export async function invalidateChannelState(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string | null | undefined,
) {
  await queryClient.invalidateQueries({ queryKey: channelsQueryKey });

  if (!channelId) {
    return;
  }

  await Promise.all([
    queryClient.invalidateQueries({
      queryKey: channelDetailQueryKey(channelId),
    }),
    queryClient.invalidateQueries({
      queryKey: channelMembersQueryKey(channelId),
    }),
  ]);
}

function setChannelArchivedState(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string,
  archivedAt: string | null,
) {
  queryClient.setQueryData<Channel[]>(channelsQueryKey, (current = []) =>
    sortChannels(
      current.map((channel) =>
        channel.id === channelId ? { ...channel, archivedAt } : channel,
      ),
    ),
  );

  queryClient.setQueryData<ChannelDetail | undefined>(
    channelDetailQueryKey(channelId),
    (current) => (current ? { ...current, archivedAt } : current),
  );
}

/**
 * Overlays `last_messages` timestamps onto channels without creating new object
 * references for channels whose `lastMessageAt` is unchanged. Preserving
 * references lets React Query's structural sharing skip re-renders for
 * channels that received no new messages. Exported for unit testing.
 */
export function applyLastMessages(
  channels: Channel[],
  lastMessages: Record<string, string>,
): Channel[] {
  return channels.map((channel) => {
    const newTs = lastMessages[channel.id] ?? null;
    if (channel.lastMessageAt === newTs) {
      return channel;
    }
    return { ...channel, lastMessageAt: newTs };
  });
}

/**
 * A failed identity read must disable persisted snapshots, not the live channel
 * request. The backend still resolves its authoritative current identity.
 */
export function canFetchChannelsForIdentity(
  ownerPubkey: string | null,
  identityReadFailed: boolean,
): boolean {
  return ownerPubkey !== null || identityReadFailed;
}

/** A hashless retry must return a full list before it can become authoritative. */
export function requireFullChannelList(channels: Channel[] | null): Channel[] {
  if (channels === null) {
    throw new Error("get_channels returned no list for a hashless request");
  }
  return channels;
}

export type RefreshChannelsQueryOptions = {
  queryClient: QueryClient;
  initialSnapshotPair: ChannelSnapshot | null;
  relayUrl: string | null;
  ownerPubkey: string | null;
  fetchChannels?: (knownHash: string | null) => Promise<GetChannelsPayload>;
  persistSnapshot?: typeof writeChannelSnapshot;
};

/**
 * Revalidates the channel query while preserving live recency updates that land
 * during the request. Exported so the production query/cache interleaving can
 * be regression-tested without replacing it with a helper-only simulation.
 */
export async function refreshChannelsQuery({
  queryClient,
  initialSnapshotPair,
  relayUrl,
  ownerPubkey,
  fetchChannels = getChannels,
  persistSnapshot = writeChannelSnapshot,
}: RefreshChannelsQueryOptions): Promise<Channel[]> {
  // Revalidation uses only an authoritative list/hash pair. The displayed
  // channels cache is intentionally ignored because successful mutations
  // patch it before the relay's list/hash has necessarily caught up.
  const cachedPair =
    queryClient.getQueryData<ChannelSnapshot>(channelsSnapshotPairKey) ??
    initialSnapshotPair;
  const knownHash = cachedPair?.hash ?? null;

  const channelsAtRequestStart =
    queryClient.getQueryData<Channel[]>(channelsQueryKey);
  const payload = await fetchChannels(knownHash);

  // A not-modified response is usable only when it echoes the exact hash
  // that described the available list. Any other hash/list pairing fails
  // slow-never-wrong by retrying without a hash.
  const hasMatchingNotModifiedResponse =
    payload.channels === null &&
    knownHash !== null &&
    payload.hash === knownHash;
  const pairChannels =
    payload.channels ??
    (hasMatchingNotModifiedResponse ? cachedPair?.channels : undefined);

  if (!pairChannels) {
    // Missing cache or a mismatched not-modified response: discard the hash
    // and fetch a complete authoritative list before updating persistence.
    const full = await fetchChannels(null);
    const authoritativeChannels = sortChannels(
      applyLastMessages(
        requireFullChannelList(full.channels),
        full.lastMessages,
      ),
    );
    const displayedAtSettlement =
      queryClient.getQueryData<Channel[]>(channelsQueryKey);
    const sorted = sortChannels(
      mergeConcurrentChannelRecency(
        authoritativeChannels,
        displayedAtSettlement,
        channelsAtRequestStart,
      ),
    );
    const pair = { channels: authoritativeChannels, hash: full.hash };
    queryClient.setQueryData(channelsSnapshotPairKey, pair);
    if (relayUrl && ownerPubkey) {
      persistSnapshot(relayUrl, ownerPubkey, pair.channels, pair.hash);
    }
    return sorted;
  }

  const authoritativeChannels = sortChannels(
    applyLastMessages(pairChannels, payload.lastMessages),
  );
  const pair = {
    channels: authoritativeChannels,
    hash: payload.hash,
  };
  queryClient.setQueryData(channelsSnapshotPairKey, pair);
  // Merge against the displayed cache at settlement so a newer live
  // timestamp cannot be rolled back by an older request result. This is
  // required for both full-list and matching not-modified responses.
  const displayedAtSettlement =
    queryClient.getQueryData<Channel[]>(channelsQueryKey);
  const refreshedForDisplay =
    payload.channels === null
      ? sortChannels(
          applyLastMessages(
            displayedAtSettlement ?? authoritativeChannels,
            payload.lastMessages,
          ),
        )
      : authoritativeChannels;
  const sorted = sortChannels(
    mergeConcurrentChannelRecency(
      refreshedForDisplay,
      displayedAtSettlement,
      channelsAtRequestStart,
    ),
  );
  if (relayUrl && ownerPubkey) {
    persistSnapshot(relayUrl, ownerPubkey, pair.channels, pair.hash);
  }
  return sorted;
}

export function useChannelsQuery(options?: { enabled?: boolean }) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? null;
  // CommunityQueryProvider remounts its QueryClient for every community. Only
  // the active identity may authorize a persisted snapshot: Community.pubkey
  // is creation-time display metadata and can be stale after identity changes.
  const identityQuery = useIdentityQuery();
  const ownerPubkey = identityQuery.data?.pubkey ?? null;
  const queryClient = useQueryClient();
  const snapshotRead = React.useMemo(
    () =>
      relayUrl && ownerPubkey
        ? inspectChannelSnapshot(relayUrl, ownerPubkey)
        : null,
    [ownerPubkey, relayUrl],
  );
  const snapshot = snapshotRead?.snapshot ?? null;
  const initialSnapshotPair = React.useMemo(
    () =>
      snapshot
        ? { channels: sortChannels(snapshot.channels), hash: snapshot.hash }
        : null,
    [snapshot],
  );
  React.useEffect(() => {
    if (relayUrl && ownerPubkey && snapshotRead && options?.enabled !== false) {
      markSnapshotDiagnostic(relayUrl, ownerPubkey, snapshotRead.diagnostics);
    }
  }, [options?.enabled, ownerPubkey, relayUrl, snapshotRead]);
  const refetchInterval = useFocusedRefetchInterval(
    CHANNELS_REFETCH_INTERVAL_MS,
  );

  const query = useQuery({
    enabled:
      (options?.enabled ?? true) &&
      relayUrl !== null &&
      canFetchChannelsForIdentity(ownerPubkey, identityQuery.isError),
    queryKey: channelsQueryKey,
    queryFn: () =>
      refreshChannelsQuery({
        queryClient,
        initialSnapshotPair,
        relayUrl,
        ownerPubkey,
      }),
    // Paint the complete persisted list immediately. `initialDataUpdatedAt: 0`
    // deliberately keeps it stale so every boot still validates against the
    // relay; queryFn reads the matching hash from the same atomic document.
    initialData: initialSnapshotPair?.channels,
    initialDataUpdatedAt: 0,
    refetchInterval,
    ...channelsFocusRefetchPolicy,
  });

  React.useEffect(() => {
    if (
      relayUrl &&
      ownerPubkey &&
      query.isSuccess &&
      query.fetchStatus === "idle" &&
      query.dataUpdatedAt > 0
    ) {
      measureFullSidebarPaint(relayUrl, ownerPubkey, query.data.length);
    }
  }, [
    query.data,
    query.dataUpdatedAt,
    query.fetchStatus,
    query.isSuccess,
    ownerPubkey,
    relayUrl,
  ]);

  return query;
}

export function useCreateChannelMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateChannelInput) => createChannel(input),
    onSuccess: (createdChannel) => {
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, createdChannel),
      );
    },
    onSettled: () => {
      // refetchType "none": onSuccess already cached the relay-returned channel;
      // an immediate getChannels() refetch blocked the dialog and could clobber
      // it with a read-after-write-lagged snapshot. Live updates reconcile later.
      void queryClient.invalidateQueries({
        queryKey: channelsQueryKey,
        refetchType: "none",
      });
    },
  });
}

export function useOpenDmMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: OpenDmInput) => openDm(input),
    onSuccess: (openedChannel) => {
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, openedChannel),
      );
    },
    onSettled: () => {
      // The relay-returned DM is already in the cache. Mark the list stale so
      // the normal live/poll refresh can reconcile it later without putting a
      // full get_channels round-trip on the critical path to the conversation.
      void queryClient.invalidateQueries({
        queryKey: channelsQueryKey,
        refetchType: "none",
      });
    },
  });
}

/**
 * Reasserts a relay-returned channel in the shared cache before a caller
 * depends on it for navigation. The open-DM mutation already made the relay
 * write authoritative, so cancel any older list read and stay local rather
 * than blocking on a read-after-write channel-list refresh.
 */
export function useUpsertCachedChannel() {
  const queryClient = useQueryClient();

  return React.useCallback(
    async (channel: Channel) => {
      await queryClient.cancelQueries({
        queryKey: channelsQueryKey,
        exact: true,
      });
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        reconcileRefreshedCachedChannel(current, channel),
      );
    },
    [queryClient],
  );
}

export function useHideDmMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (channelId: string) => hideDm(channelId),
    onMutate: async (channelId) => {
      await queryClient.cancelQueries({ queryKey: channelsQueryKey });
      const previous = queryClient.getQueryData<Channel[]>(channelsQueryKey);
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current = []) =>
        current.filter((channel) => channel.id !== channelId),
      );
      return { previous };
    },
    onError: (_error, _channelId, context) => {
      if (context?.previous) {
        queryClient.setQueryData(channelsQueryKey, context.previous);
      }
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    },
  });
}

export function useChannelDetailsQuery(
  channelId: string | null,
  enabled = true,
) {
  return useQuery({
    enabled: enabled && channelId !== null,
    queryKey: ["channels", channelId ?? "none", "detail"],
    queryFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return getChannelDetails(channelId);
    },
    staleTime: 30_000,
  });
}

export function useChannelMembersQuery(
  channelId: string | null,
  enabled = true,
) {
  return useQuery({
    enabled: enabled && channelId !== null,
    queryKey: ["channels", channelId ?? "none", "members"],
    queryFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return getChannelMembers(channelId);
    },
    staleTime: CHANNEL_MEMBERS_STALE_TIME_MS,
  });
}

export function useUpdateChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: Omit<UpdateChannelInput, "channelId">) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return updateChannel({ ...input, channelId });
    },
    onMutate: () => ({ channelId }),
    onSuccess: (updatedChannel) => {
      queryClient.setQueryData<ChannelDetail>(
        channelDetailQueryKey(updatedChannel.id),
        updatedChannel,
      );
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current = []) =>
        sortChannels(
          current.map((channel) =>
            channel.id === updatedChannel.id ? updatedChannel : channel,
          ),
        ),
      );
    },
    onSettled: (_data, _error, _variables, context) => {
      // refetchType "none": onSuccess already cached the relay-returned detail;
      // awaiting the full channel-list refetch kept the edit dialog stuck on
      // "Saving..." (same failure #1360 fixed for create).
      void queryClient.invalidateQueries({
        queryKey: channelsQueryKey,
        refetchType: "none",
      });
      if (context?.channelId) {
        void queryClient.invalidateQueries({
          queryKey: channelDetailQueryKey(context.channelId),
          refetchType: "none",
        });
      }
    },
  });
}

export function useSetChannelTopicMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: Omit<SetChannelTopicInput, "channelId">) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return setChannelTopic({ ...input, channelId });
    },
    onSettled: () => {
      // fire-and-forget: awaiting the channels-list refetch blocks the dialog
      void invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useSetChannelPurposeMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: Omit<SetChannelPurposeInput, "channelId">) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return setChannelPurpose({ ...input, channelId });
    },
    onSettled: () => {
      // fire-and-forget: awaiting the channels-list refetch blocks the dialog
      void invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useArchiveChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await archiveChannel(channelId);
    },
    onSuccess: () => {
      if (!channelId) {
        return;
      }

      setChannelArchivedState(queryClient, channelId, new Date().toISOString());
    },
    onSettled: async () => {
      await invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useUnarchiveChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await unarchiveChannel(channelId);
    },
    onSuccess: () => {
      if (!channelId) {
        return;
      }

      setChannelArchivedState(queryClient, channelId, null);
    },
    onSettled: async () => {
      await invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useDeleteChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await deleteChannel(channelId);
    },
    onSuccess: () => {
      if (!channelId) {
        return;
      }

      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current = []) =>
        current.filter((channel) => channel.id !== channelId),
      );
      queryClient.removeQueries({
        queryKey: channelDetailQueryKey(channelId),
      });
      queryClient.removeQueries({
        queryKey: channelMembersQueryKey(channelId),
      });
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
        queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
        queryClient.invalidateQueries({ queryKey: ["relay-agents"] }),
      ]);
    },
  });
}

export function useAddChannelMembersMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (
      input: Omit<AddChannelMembersInput, "channelId"> & {
        channelId?: string;
      },
    ) => {
      const { channelId: capturedChannelId, ...rest } = input;
      const effectiveChannelId = capturedChannelId ?? channelId;
      if (!effectiveChannelId) {
        throw new Error("No channel selected.");
      }

      return addChannelMembers({ ...rest, channelId: effectiveChannelId });
    },
    onSuccess: (result, variables) => {
      const effectiveChannelId = variables.channelId ?? channelId;
      if (
        effectiveChannelId &&
        variables.role === "bot" &&
        result.added.length > 0
      ) {
        void invokeTauri("sync_agents_to_active_huddle", {
          channelId: effectiveChannelId,
          agentPubkeys: result.added,
        }).catch((error) => {
          console.warn("Could not sync added agents into Huddle:", error);
        });
      }
    },
    onSettled: async (_data, _err, variables) => {
      // Invalidate the effective channel (the one actually mutated) not the
      // live hook-closure channel, which may have changed mid-send.
      const effectiveChannelId = variables?.channelId ?? channelId;
      await Promise.all([
        invalidateChannelState(queryClient, effectiveChannelId),
        // Relay-agent discovery is membership-backed. Without this refresh,
        // adding an existing agent through the members picker leaves the
        // autocomplete directory stale and the same-named local persona can
        // be offered as a new agent until the five-minute focus refresh.
        ...(variables?.role === "bot"
          ? [
              queryClient.invalidateQueries({
                queryKey: ["relay-agents"],
              }),
            ]
          : []),
      ]);
    },
  });
}

export function useRemoveChannelMemberMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (pubkey: string) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await removeChannelMember(channelId, pubkey);
    },
    onSettled: async () => {
      await Promise.all([
        invalidateChannelState(queryClient, channelId),
        queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
        queryClient.invalidateQueries({ queryKey: ["relay-agents"] }),
      ]);
    },
  });
}

export function useJoinChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await joinChannel(channelId);
    },
    onSettled: async () => {
      await invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useLeaveChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await leaveChannel(channelId);
    },
    onSettled: async () => {
      await invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useSelectedChannel(
  channels: Channel[],
  preferredChannelId: string | null,
) {
  const [selectedChannelId, setSelectedChannelId] = React.useState<
    string | null
  >(preferredChannelId);

  const selectedChannel = React.useMemo(
    () =>
      channels.find((channel) => channel.id === selectedChannelId) ??
      channels.find((channel) => channel.channelType !== "forum") ??
      channels[0] ??
      null,
    [channels, selectedChannelId],
  );

  React.useEffect(() => {
    if (!selectedChannel && channels.length === 0) {
      return;
    }

    if (!selectedChannelId && selectedChannel) {
      setSelectedChannelId(selectedChannel.id);
      return;
    }

    if (
      selectedChannelId &&
      !channels.some((channel) => channel.id === selectedChannelId) &&
      selectedChannel
    ) {
      setSelectedChannelId(selectedChannel.id);
    }
  }, [channels, selectedChannel, selectedChannelId]);

  return {
    selectedChannel,
    selectedChannelId,
    setSelectedChannelId,
  };
}

// ── Canvas ────────────────────────────────────────────────────────────────────
export function useCanvasQuery(channelId: string | null, enabled = true) {
  return useQuery({
    queryKey: ["channel-canvas", channelId],
    queryFn: () => {
      if (!channelId) {
        return Promise.reject(new Error("No channel selected"));
      }
      return getCanvas(channelId);
    },
    enabled: enabled && channelId !== null,
  });
}

export function useSetCanvasMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (content: string) => {
      if (!channelId) {
        return Promise.reject(new Error("No channel selected"));
      }
      return setCanvas({ channelId, content });
    },
    onSuccess: () => {
      if (channelId) {
        void queryClient.invalidateQueries({
          queryKey: ["channel-canvas", channelId],
        });
      }
    },
  });
}
