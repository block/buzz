import { FolderKanban, Hash } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  collectProjectRelatedChannelRows,
  projectRelatedChannelRowKey,
} from "@/features/projects/lib/projectRelatedChannels";
import { selectionItemFromChannel } from "@/features/projects/lib/projectSelection";
import { listRowDescription } from "@/features/projects/lib/projectsViewHelpers";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { ProjectEntityListRow } from "./ProjectEntityListRow";
import { ProjectSelectableGroup } from "./ProjectSelectableGroup";

function lastMessageAtSeconds(value: string | null | undefined) {
  if (!value) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? Math.floor(ms / 1_000) : null;
}

export function ProjectsChannelsList({ projects }: { projects: Project[] }) {
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery({ enabled: projects.length > 0 });
  const channelsById = React.useMemo(() => {
    return new Map(
      (channelsQuery.data ?? []).map((channel) => [channel.id, channel]),
    );
  }, [channelsQuery.data]);
  const rows = React.useMemo(() => {
    const collected = collectProjectRelatedChannelRows(projects);
    return [...collected].sort((left, right) => {
      const leftChannel = channelsById.get(left.channelId);
      const rightChannel = channelsById.get(right.channelId);
      const leftName = leftChannel?.name ?? left.channelId;
      const rightName = rightChannel?.name ?? right.channelId;
      const leftActivity =
        lastMessageAtSeconds(leftChannel?.lastMessageAt) ?? 0;
      const rightActivity =
        lastMessageAtSeconds(rightChannel?.lastMessageAt) ?? 0;
      return (
        rightActivity - leftActivity ||
        leftName.localeCompare(rightName) ||
        left.projectName.localeCompare(right.projectName) ||
        (left.repositoryName ?? "").localeCompare(right.repositoryName ?? "") ||
        left.channelId.localeCompare(right.channelId)
      );
    });
  }, [channelsById, projects]);
  const participantPubkeys = React.useMemo(
    () => [
      ...new Set(
        rows.flatMap((row) => {
          const channel = channelsById.get(row.channelId);
          return channel?.participantPubkeys ?? channel?.participants ?? [];
        }),
      ),
    ],
    [channelsById, rows],
  );
  const profilesQuery = useUsersBatchQuery(participantPubkeys, {
    enabled: participantPubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const selectionItemsByRowKey = React.useMemo(() => {
    const items = new Map<
      string,
      ReturnType<typeof selectionItemFromChannel>
    >();
    for (const row of rows) {
      const channel = channelsById.get(row.channelId);
      const name = channel?.name ?? row.channelId.slice(0, 8);
      const rowKey = projectRelatedChannelRowKey(row);
      const item = selectionItemFromChannel({
        channelId: row.channelId,
        people: channel?.participantPubkeys ?? channel?.participants ?? [],
        title: `#${name}`,
      });
      items.set(rowKey, { ...item, id: `${item.id}:${rowKey}` });
    }
    return items;
  }, [channelsById, rows]);
  const rangeItems = React.useMemo(
    () => [...selectionItemsByRowKey.values()],
    [selectionItemsByRowKey],
  );
  const groups = React.useMemo(() => {
    const grouped = new Map<
      string,
      { projectId: string; projectName: string; rows: typeof rows }
    >();
    for (const row of rows) {
      const existing = grouped.get(row.projectId);
      if (existing) {
        existing.rows.push(row);
        continue;
      }
      grouped.set(row.projectId, {
        projectId: row.projectId,
        projectName: row.projectName,
        rows: [row],
      });
    }
    return [...grouped.values()];
  }, [rows]);

  if (channelsQuery.isLoading && rows.length === 0) {
    return <BuzzLoadingState label="Loading project channels" />;
  }
  if (rows.length === 0) {
    return (
      <p className="px-4 py-6 text-sm text-muted-foreground">
        No channels are bound to these projects yet. Link a discussion channel
        to a project or repository and it will show up here.
      </p>
    );
  }

  return (
    <div className="space-y-2" data-testid="projects-channels-list">
      {groups.map((group) => (
        <ProjectSelectableGroup
          count={group.rows.length}
          groupKey={group.projectId}
          headerTestId="projects-channel-project-group-header"
          icon={<FolderKanban className="h-4 w-4" />}
          items={group.rows.flatMap((row) => {
            const item = selectionItemsByRowKey.get(
              projectRelatedChannelRowKey(row),
            );
            return item ? [item] : [];
          })}
          key={group.projectId}
          label={group.projectName}
          labelTestId="project-channel-project"
          testId="projects-channel-project-group"
        >
          <ul className="space-y-0.5">
            {group.rows.map((row) => {
              const rowKey = projectRelatedChannelRowKey(row);
              const channel = channelsById.get(row.channelId);
              const name = channel?.name ?? row.channelId.slice(0, 8);
              const lastActivityAt = lastMessageAtSeconds(
                channel?.lastMessageAt,
              );
              const repositoryLabel =
                row.repositoryName?.trim() || "Project channel";
              const people =
                channel?.participantPubkeys ?? channel?.participants ?? [];
              const selectionItem = selectionItemsByRowKey.get(rowKey);
              return (
                <li key={rowKey}>
                  <ProjectEntityListRow
                    affiliation={
                      <span data-testid="project-channel-repository">
                        {repositoryLabel}
                      </span>
                    }
                    affiliationTitle={`${group.projectName} · ${repositoryLabel}`}
                    count={channel?.memberCount}
                    countTestId="project-channel-message-count"
                    countTitle={
                      channel
                        ? `${channel.memberCount} ${
                            channel.memberCount === 1 ? "member" : "members"
                          }`
                        : undefined
                    }
                    dateSeconds={lastActivityAt}
                    dateTestId="project-channel-row-date"
                    description={listRowDescription(channel?.description, name)}
                    icon={
                      <Hash className="h-3.5 w-3.5 text-muted-foreground/70" />
                    }
                    onClick={() => void goChannel(row.channelId)}
                    people={people}
                    peopleTestId="project-channel-participants"
                    profiles={profiles}
                    selection={
                      selectionItem
                        ? { item: selectionItem, rangeItems }
                        : undefined
                    }
                    testId="project-channel-row"
                    title={`#${name}`}
                    titleAttr={`Open #${name}`}
                  />
                </li>
              );
            })}
          </ul>
        </ProjectSelectableGroup>
      ))}
    </div>
  );
}
