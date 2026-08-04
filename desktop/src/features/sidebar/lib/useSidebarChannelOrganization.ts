import * as React from "react";

import type { Channel } from "@/shared/api/types";
import {
  sectionSortGroupKey,
  sortChannelsForSidebar,
  type ChannelSortGroupKey,
  type ChannelSortMode,
} from "./channelSortPreference";
import { useChannelManualOrder } from "./useChannelManualOrder";
import { useChannelSortPreference } from "./useChannelSortPreference";
import type { ChannelSection } from "./useChannelSections";

type OrganizationInput = {
  pubkey?: string;
  relayUrl?: string;
  channels: Channel[];
  sections: ChannelSection[];
  assignments: Record<string, string>;
  starredChannelIds?: ReadonlySet<string>;
  assignChannel: (channelId: string, sectionId: string) => void;
  unassignChannel: (channelId: string) => void;
};

export function useSidebarChannelOrganization({
  pubkey,
  relayUrl,
  channels,
  sections,
  assignments,
  starredChannelIds,
  assignChannel,
  unassignChannel,
}: OrganizationInput) {
  const sectionIds = React.useMemo(
    () => sections.map((section) => section.id),
    [sections],
  );
  const { sortModeFor: persistedSortModeFor, setSortModeFor } =
    useChannelSortPreference(pubkey, relayUrl, sectionIds);
  const manualOrder = useChannelManualOrder(pubkey, relayUrl, sectionIds);
  const sortModeFor = React.useCallback(
    (group: ChannelSortGroupKey): ChannelSortMode =>
      manualOrder.isManual(group) ? "manual" : persistedSortModeFor(group),
    [manualOrder, persistedSortModeFor],
  );
  const streamChannels = React.useMemo(
    () => channels.filter((channel) => channel.channelType === "stream"),
    [channels],
  );

  const sortStreamGroup = React.useCallback(
    (group: ChannelSortGroupKey, groupChannels: Channel[]) => {
      const mode = sortModeFor(group);
      if (mode !== "manual") {
        return sortChannelsForSidebar(groupChannels, mode);
      }
      // Implicit Manual (default, no user reorder yet): deterministic A–Z.
      // First reorder / explicit Manual choice enables + seeds persistence.
      if (!manualOrder.isManual(group)) {
        return sortChannelsForSidebar(groupChannels, "alpha");
      }
      const orderedIds = manualOrder.orderIds(
        group,
        groupChannels.map((channel) => channel.id),
      );
      const byId = new Map(
        groupChannels.map((channel) => [channel.id, channel]),
      );
      return orderedIds.flatMap((id) => {
        const channel = byId.get(id);
        return channel ? [channel] : [];
      });
    },
    [manualOrder, sortModeFor],
  );

  const sectionBuckets = React.useMemo(() => {
    const bySection: Record<string, Channel[]> = {};
    const unassigned: Channel[] = [];
    const liveSectionIds = new Set(sections.map((section) => section.id));
    for (const channel of streamChannels) {
      if (starredChannelIds?.has(channel.id)) continue;
      const sectionId = assignments[channel.id];
      if (sectionId && liveSectionIds.has(sectionId)) {
        if (!bySection[sectionId]) bySection[sectionId] = [];
        bySection[sectionId].push(channel);
      } else {
        unassigned.push(channel);
      }
    }
    for (const sectionId of Object.keys(bySection)) {
      bySection[sectionId] = sortStreamGroup(
        sectionSortGroupKey(sectionId),
        bySection[sectionId],
      );
    }
    return {
      bySection,
      unassigned: sortStreamGroup("channels", unassigned),
    };
  }, [
    assignments,
    sections,
    sortStreamGroup,
    starredChannelIds,
    streamChannels,
  ]);

  const handleSortModeChange = React.useCallback(
    (
      group: ChannelSortGroupKey,
      mode: ChannelSortMode,
      visibleChannels: Channel[],
    ) => {
      if (mode === "manual") {
        manualOrder.seedOrder(
          group,
          visibleChannels.map((channel) => channel.id),
        );
        manualOrder.setManualMode(group, true);
        return;
      }
      manualOrder.setManualMode(group, false);
      setSortModeFor(group, mode);
    },
    [manualOrder, setSortModeFor],
  );

  const groupChannelIds = React.useMemo(() => {
    const groups: Record<string, string[]> = {
      channels: sectionBuckets.unassigned.map((channel) => channel.id),
    };
    for (const section of sections) {
      groups[sectionSortGroupKey(section.id)] = (
        sectionBuckets.bySection[section.id] ?? []
      ).map((channel) => channel.id);
    }
    return groups;
  }, [sections, sectionBuckets]);

  const manualGroupKeys = React.useMemo(
    () =>
      new Set<ChannelSortGroupKey>(
        [
          "channels",
          ...sections.map((section) => sectionSortGroupKey(section.id)),
        ].filter(
          (group): group is ChannelSortGroupKey =>
            sortModeFor(group as ChannelSortGroupKey) === "manual",
        ),
      ),
    [sections, sortModeFor],
  );

  const handleMoveChannel = React.useCallback(
    (input: {
      channelId: string;
      sourceGroup: ChannelSortGroupKey;
      targetGroup: ChannelSortGroupKey;
      overChannelId?: string;
    }) => {
      const { channelId, sourceGroup, targetGroup, overChannelId } = input;
      if (sourceGroup !== targetGroup) {
        if (targetGroup === "channels") {
          unassignChannel(channelId);
        } else if (targetGroup.startsWith("section:")) {
          assignChannel(channelId, targetGroup.slice("section:".length));
        }
      }
      manualOrder.moveChannel({
        channelId,
        sourceGroup,
        targetGroup,
        ...(overChannelId ? { overChannelId } : {}),
        sourceLiveIds: groupChannelIds[sourceGroup] ?? [],
        targetLiveIds: groupChannelIds[targetGroup] ?? [],
      });
    },
    [assignChannel, groupChannelIds, manualOrder, unassignChannel],
  );

  const preserveDeletedSectionOrder = React.useCallback(
    (sectionId: string) => {
      manualOrder.mergeDeletedSection(
        sectionId,
        (sectionBuckets.bySection[sectionId] ?? []).map(
          (channel) => channel.id,
        ),
        sectionBuckets.unassigned.map((channel) => channel.id),
      );
    },
    [manualOrder, sectionBuckets],
  );

  return {
    sectionIds,
    streamChannels,
    sectionBuckets,
    sortModeFor,
    setSortModeFor,
    handleSortModeChange,
    manualGroupKeys,
    handleMoveChannel,
    preserveDeletedSectionOrder,
  };
}
