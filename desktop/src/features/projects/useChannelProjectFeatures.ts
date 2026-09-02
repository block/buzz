import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import { useChannelSections } from "@/features/sidebar/lib/useChannelSections";
import type { Channel } from "@/shared/api/types";

import {
  useProjectActivitySummariesQuery,
  useProjectIssuesQuery,
  useProjectsQuery,
} from "./hooks";
import {
  CHANNEL_PROJECT_FEATURES_CHANGED_EVENT,
  channelProjectFeatureEnabled,
  channelProjectFeatureStorageKey,
  findChannelProject,
  projectPrimaryRepository,
  projectRelatedChannelIds,
  projectRelatedRepositories,
  readChannelProjectFeaturePreferences,
  type ChannelProjectFeature,
  type ChannelProjectFeaturePreferences,
  writeChannelProjectFeaturePreferences,
} from "./channelProjectFeatures";

export function useChannelProjectFeatures({
  channel,
  currentPubkey,
  relayUrl,
}: {
  channel: Channel;
  currentPubkey?: string;
  relayUrl?: string;
}) {
  const [preferences, setPreferences] =
    React.useState<ChannelProjectFeaturePreferences>(() =>
      readChannelProjectFeaturePreferences(currentPubkey, relayUrl, channel.id),
    );
  const projectsQuery = useProjectsQuery(Boolean(currentPubkey && relayUrl));
  const channelsQuery = useChannelsQuery();
  const channelSections = useChannelSections(currentPubkey, relayUrl);
  const project = findChannelProject(projectsQuery.data ?? [], channel.id);
  const primaryRepository = projectPrimaryRepository(project);
  const relatedRepositories = projectRelatedRepositories(project);
  const relatedChannelIds = projectRelatedChannelIds(project, channel.id);
  const issuesQuery = useProjectIssuesQuery(primaryRepository);
  const projectActivityQuery = useProjectActivitySummariesQuery(
    project ? [project] : [],
  );
  const projectActivity = project
    ? projectActivityQuery.data?.[project.id]
    : undefined;
  const breakoutSection =
    channelSections.sections.find(
      (section) => section.id === preferences.breakoutSectionId,
    ) ?? null;
  const localBreakoutChannelIds = breakoutSection
    ? Object.entries(channelSections.assignments)
        .filter(
          ([channelId, sectionId]) =>
            sectionId === breakoutSection.id && channelId !== channel.id,
        )
        .map(([channelId]) => channelId)
    : [];
  const breakoutChannelIds = [
    ...new Set([...relatedChannelIds, ...localBreakoutChannelIds]),
  ];
  const existing = {
    tasks: (issuesQuery.data?.length ?? 0) > 0,
    breakouts: breakoutChannelIds.length > 0,
    reviews: (projectActivity?.prCount ?? 0) > 0,
    repositories: relatedRepositories.length > 0,
  };
  const enabled = {
    tasks: channelProjectFeatureEnabled({
      feature: "tasks",
      hasExistingData: existing.tasks,
      preferences,
    }),
    breakouts: channelProjectFeatureEnabled({
      feature: "breakouts",
      hasExistingData: existing.breakouts,
      preferences,
    }),
    reviews: channelProjectFeatureEnabled({
      feature: "reviews",
      hasExistingData: existing.reviews,
      preferences,
    }),
    repositories: channelProjectFeatureEnabled({
      feature: "repositories",
      hasExistingData: existing.repositories,
      preferences,
    }),
  };

  React.useEffect(() => {
    setPreferences(
      readChannelProjectFeaturePreferences(currentPubkey, relayUrl, channel.id),
    );
  }, [channel.id, currentPubkey, relayUrl]);

  React.useEffect(() => {
    if (!currentPubkey || !relayUrl) return;
    const key = channelProjectFeatureStorageKey(currentPubkey, relayUrl);
    const handleChange = (event: Event) => {
      if (
        event.type === CHANNEL_PROJECT_FEATURES_CHANGED_EVENT &&
        (event as CustomEvent<{ key?: string }>).detail?.key !== key
      ) {
        return;
      }
      if (event.type === "storage" && (event as StorageEvent).key !== key) {
        return;
      }
      setPreferences(
        readChannelProjectFeaturePreferences(
          currentPubkey,
          relayUrl,
          channel.id,
        ),
      );
    };
    window.addEventListener("storage", handleChange);
    window.addEventListener(
      CHANNEL_PROJECT_FEATURES_CHANGED_EVENT,
      handleChange,
    );
    return () => {
      window.removeEventListener("storage", handleChange);
      window.removeEventListener(
        CHANNEL_PROJECT_FEATURES_CHANGED_EVENT,
        handleChange,
      );
    };
  }, [channel.id, currentPubkey, relayUrl]);

  const updatePreferences = React.useCallback(
    (patch: Partial<ChannelProjectFeaturePreferences>) => {
      if (!currentPubkey || !relayUrl) return null;
      const next = writeChannelProjectFeaturePreferences(
        currentPubkey,
        relayUrl,
        channel.id,
        patch,
      );
      if (next) setPreferences(next);
      return next;
    },
    [channel.id, currentPubkey, relayUrl],
  );

  const ensureBreakoutSection = React.useCallback(() => {
    let section =
      channelSections.sections.find(
        (candidate) => candidate.id === preferences.breakoutSectionId,
      ) ?? null;
    if (!section) {
      section = channelSections.createSection(channel.name);
    }
    if (!section) return null;
    if (channelSections.assignments[channel.id] !== section.id) {
      channelSections.assignChannel(channel.id, section.id);
    }
    for (const channelId of relatedChannelIds) {
      if (channelSections.assignments[channelId] !== section.id) {
        channelSections.assignChannel(channelId, section.id);
      }
    }
    if (preferences.breakoutSectionId !== section.id) {
      updatePreferences({ breakoutSectionId: section.id });
    }
    return section;
  }, [
    channel.id,
    channel.name,
    channelSections,
    preferences.breakoutSectionId,
    relatedChannelIds,
    updatePreferences,
  ]);

  const setFeatureEnabled = React.useCallback(
    (feature: ChannelProjectFeature, value: boolean) => {
      updatePreferences({ [feature]: value });
      if (feature === "breakouts") {
        if (value) {
          ensureBreakoutSection();
        } else if (breakoutSection && !existing.breakouts) {
          channelSections.deleteSection(breakoutSection.id);
          updatePreferences({ breakoutSectionId: undefined });
        }
      }
    },
    [
      breakoutSection,
      channelSections,
      ensureBreakoutSection,
      existing.breakouts,
      updatePreferences,
    ],
  );

  return {
    breakoutChannelIds,
    breakoutSection,
    channelSections,
    channels: channelsQuery.data ?? [],
    enabled,
    existing,
    issuesQuery,
    projectActivityQuery,
    preferences,
    primaryRepository,
    project,
    projects: projectsQuery.data ?? [],
    projectsQuery,
    relatedChannelIds,
    relatedRepositories,
    ensureBreakoutSection,
    setFeatureEnabled,
  };
}
