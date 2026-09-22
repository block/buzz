import { FolderKanban, Hash, LockKeyhole } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  collapseProjectRelatedChannelRows,
  collectProjectRelatedChannelRows,
  projectRelatedChannelDisplayRowKey,
} from "@/features/projects/lib/projectRelatedChannels";
import { selectionItemFromChannel } from "@/features/projects/lib/projectSelection";
import { matchesProjectsSearch } from "@/features/projects/lib/projectsSearch";
import { listRowDescription } from "@/features/projects/lib/projectsViewHelpers";
import { useTranslation } from "@/i18n";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { ProjectEntityListRow } from "./ProjectEntityListRow";
import { ProjectSelectableGroup } from "./ProjectSelectableGroup";
import { ProjectPanelState } from "./ProjectPanelState";

function lastMessageAtSeconds(value: string | null | undefined) {
  if (!value) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? Math.floor(ms / 1_000) : null;
}

export function ProjectsChannelsList({
  projects,
  searchQuery = "",
}: {
  projects: Project[];
  searchQuery?: string;
}) {
  const { t } = useTranslation();
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery({ enabled: projects.length > 0 });
  const channelsById = React.useMemo(() => {
    return new Map(
      (channelsQuery.data ?? []).map((channel) => [channel.id, channel]),
    );
  }, [channelsQuery.data]);
  const rows = React.useMemo(() => {
    const collected = collapseProjectRelatedChannelRows(
      collectProjectRelatedChannelRows(projects),
    );
    return collected
      .filter((row) => {
        const channel = channelsById.get(row.channelId);
        return matchesProjectsSearch(searchQuery, [
          channel?.name,
          channel?.description,
          row.projectName,
          ...row.repositoryNames,
        ]);
      })
      .sort((left, right) => {
        const leftChannel = channelsById.get(left.channelId);
        const rightChannel = channelsById.get(right.channelId);
        const leftName =
          leftChannel?.name ?? t("projects.channels-list.channel-unavailable");
        const rightName =
          rightChannel?.name ?? t("projects.channels-list.channel-unavailable");
        const leftActivity =
          lastMessageAtSeconds(leftChannel?.lastMessageAt) ?? 0;
        const rightActivity =
          lastMessageAtSeconds(rightChannel?.lastMessageAt) ?? 0;
        return (
          rightActivity - leftActivity ||
          leftName.localeCompare(rightName) ||
          left.projectName.localeCompare(right.projectName) ||
          left.repositoryNames
            .join(",")
            .localeCompare(right.repositoryNames.join(",")) ||
          left.channelId.localeCompare(right.channelId)
        );
      });
  }, [channelsById, projects, searchQuery, t]);
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
      const name =
        channel?.name ?? t("projects.channels-list.channel-unavailable");
      const rowKey = projectRelatedChannelDisplayRowKey(row);
      const item = selectionItemFromChannel({
        channelId: row.channelId,
        people: channel?.participantPubkeys ?? channel?.participants ?? [],
        title: channel ? `#${name}` : name,
      });
      items.set(rowKey, { ...item, id: `${item.id}:${rowKey}` });
    }
    return items;
  }, [channelsById, rows, t]);
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
    return <BuzzLoadingState label={t("projects.channels-list.loading")} />;
  }
  if (rows.length === 0) {
    const searching = Boolean(searchQuery.trim());
    return (
      <ProjectPanelState
        description={
          searching
            ? t("projects.shared.try-different-search")
            : t("projects.channels-list.empty-description")
        }
        panel={false}
        title={
          searching
            ? t("projects.channels-list.no-matching-title")
            : t("projects.channels-list.empty-title")
        }
      />
    );
  }

  return (
    <div className="space-y-2" data-testid="projects-channels-list">
      {groups.map((group) => (
        <ProjectSelectableGroup
          count={group.rows.length}
          groupKey={group.projectId}
          headerClassName="mx-0 gap-3 px-4"
          headerTestId="projects-channel-project-group-header"
          icon={<FolderKanban className="h-4 w-4" />}
          items={group.rows.flatMap((row) => {
            const item = selectionItemsByRowKey.get(
              projectRelatedChannelDisplayRowKey(row),
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
              const rowKey = projectRelatedChannelDisplayRowKey(row);
              const channel = channelsById.get(row.channelId);
              const name =
                channel?.name ??
                t("projects.channels-list.channel-unavailable");
              const lastActivityAt = lastMessageAtSeconds(
                channel?.lastMessageAt,
              );
              const repositoryLabel =
                row.repositoryNames.length === 0
                  ? t("channels.type.project-channel")
                  : row.repositoryNames.length === 1
                    ? row.repositoryNames[0]
                    : t("projects.selection.count-repository", {
                        count: row.repositoryNames.length,
                      });
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
                    description={
                      channel
                        ? listRowDescription(channel.description, name)
                        : t("projects.channels-list.details-unavailable")
                    }
                    icon={
                      channel ? (
                        <Hash className="h-3.5 w-3.5 text-muted-foreground/70" />
                      ) : (
                        <LockKeyhole className="h-3.5 w-3.5 text-muted-foreground/55" />
                      )
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
                    title={channel ? `#${name}` : name}
                    titleAttr={
                      channel
                        ? t("projects.discussion-channels.open-channel-title", {
                            name,
                          })
                        : t("projects.channels-list.open-unavailable")
                    }
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
