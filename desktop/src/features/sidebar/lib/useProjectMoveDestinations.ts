import * as React from "react";

import {
  projectRelatedChannelIds,
  readChannelProjectFeaturePreferences,
  writeChannelProjectFeaturePreferences,
} from "@/features/projects/channelProjectFeatures";
import { useProjectsQuery } from "@/features/projects/hooks";
import {
  isExplicitProject,
  type Project,
} from "@/features/projects/projectModels";
import { addProjectToSidebar } from "@/features/projects/lib/projectSidebarMembership";
import type { Channel } from "@/shared/api/types";

import type { ChannelSection } from "./useChannelSections";

export type ProjectMoveDestination = {
  name: string;
  projectAddress: string;
  projectChannelId: string;
  relatedChannelIds: string[];
  sectionId: string | null;
};

export function listMemberProjectMoveDestinations({
  channels,
  projects,
  readPreferences,
  sections,
}: {
  channels: readonly Channel[];
  projects: readonly Project[];
  readPreferences: (channelId: string) => { breakoutSectionId?: string };
  sections: readonly ChannelSection[];
}): ProjectMoveDestination[] {
  const memberChannelIds = new Set(
    channels.filter((channel) => channel.isMember).map((channel) => channel.id),
  );
  const sectionIds = new Set(sections.map((section) => section.id));
  const seen = new Set<string>();

  return projects
    .filter(isExplicitProject)
    .flatMap((project) => {
      const projectChannelId = project.projectChannelId?.trim();
      if (
        !projectChannelId ||
        !memberChannelIds.has(projectChannelId) ||
        seen.has(project.projectAddress)
      ) {
        return [];
      }
      seen.add(project.projectAddress);
      const preferredSectionId =
        readPreferences(projectChannelId).breakoutSectionId;
      return [
        {
          name: project.name,
          projectAddress: project.projectAddress,
          projectChannelId,
          relatedChannelIds: projectRelatedChannelIds(
            project,
            projectChannelId,
          ),
          sectionId:
            preferredSectionId && sectionIds.has(preferredSectionId)
              ? preferredSectionId
              : null,
        },
      ];
    })
    .sort(
      (left, right) =>
        left.name.localeCompare(right.name) ||
        left.projectAddress.localeCompare(right.projectAddress),
    );
}

export function moveChannelToProjectSection({
  assignChannel,
  channelId,
  createSection,
  destination,
  sections,
}: {
  assignChannel: (channelId: string, sectionId: string) => void;
  channelId: string;
  createSection: (name: string) => ChannelSection | null;
  destination: ProjectMoveDestination;
  sections: readonly ChannelSection[];
}): ChannelSection | null {
  const existingSection = destination.sectionId
    ? sections.find((section) => section.id === destination.sectionId)
    : null;
  const section = existingSection ?? createSection(destination.name);
  if (!section) return null;

  for (const assignedChannelId of new Set([
    destination.projectChannelId,
    ...destination.relatedChannelIds,
    channelId,
  ])) {
    assignChannel(assignedChannelId, section.id);
  }
  return section;
}

export function useProjectMoveDestinations({
  assignChannel,
  channels,
  createSection,
  currentPubkey,
  relayOrigin,
  relayUrl,
  sections,
}: {
  assignChannel: (channelId: string, sectionId: string) => void;
  channels: readonly Channel[];
  createSection: (name: string) => ChannelSection | null;
  currentPubkey?: string;
  relayOrigin: string | null;
  relayUrl?: string;
  sections: readonly ChannelSection[];
}) {
  const projectsQuery = useProjectsQuery(Boolean(currentPubkey && relayUrl));
  const destinations = React.useMemo(
    () =>
      listMemberProjectMoveDestinations({
        channels,
        projects: projectsQuery.data ?? [],
        readPreferences: (channelId) =>
          readChannelProjectFeaturePreferences(
            currentPubkey,
            relayUrl,
            channelId,
          ),
        sections,
      }),
    [channels, currentPubkey, projectsQuery.data, relayUrl, sections],
  );

  const assignChannelToProject = React.useCallback(
    (channelId: string, projectAddress: string) => {
      if (!currentPubkey || !relayOrigin || !relayUrl) return;
      const destination = destinations.find(
        (candidate) => candidate.projectAddress === projectAddress,
      );
      if (!destination) return;
      const section = moveChannelToProjectSection({
        assignChannel,
        channelId,
        createSection,
        destination,
        sections,
      });
      if (!section) return;
      addProjectToSidebar(
        destination.projectAddress,
        relayOrigin,
        currentPubkey,
      );
      writeChannelProjectFeaturePreferences(
        currentPubkey,
        relayUrl,
        destination.projectChannelId,
        { breakouts: true, breakoutSectionId: section.id },
      );
    },
    [
      assignChannel,
      createSection,
      currentPubkey,
      destinations,
      relayOrigin,
      relayUrl,
      sections,
    ],
  );

  return { assignChannelToProject, destinations };
}
