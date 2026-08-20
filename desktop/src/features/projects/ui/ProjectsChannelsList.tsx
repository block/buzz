import { Hash } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  collectProjectRelatedChannelRows,
  projectRelatedChannelRowKey,
} from "@/features/projects/lib/projectRelatedChannels";
import { listRowDescription } from "@/features/projects/lib/projectsViewHelpers";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { ProjectEntityListRow } from "./ProjectEntityListRow";

function lastMessageAtSeconds(value: string | null | undefined) {
  if (!value) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? Math.floor(ms / 1_000) : null;
}

function affiliationLabel(projectName: string, repositoryName: string | null) {
  if (!repositoryName || repositoryName === projectName) return projectName;
  return `${projectName} · ${repositoryName}`;
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
    <ul
      className="divide-y divide-border/60"
      data-testid="projects-channels-list"
    >
      {rows.map((row) => {
        const channel = channelsById.get(row.channelId);
        const name = channel?.name ?? row.channelId.slice(0, 8);
        const lastActivityAt = lastMessageAtSeconds(channel?.lastMessageAt);
        const affiliation = affiliationLabel(
          row.projectName,
          row.repositoryName,
        );
        const people =
          channel?.participantPubkeys ?? channel?.participants ?? [];
        return (
          <li key={projectRelatedChannelRowKey(row)}>
            <ProjectEntityListRow
              affiliation={
                <span data-testid="project-channel-project">
                  <span data-testid="project-channel-repository">
                    {affiliation}
                  </span>
                </span>
              }
              affiliationTitle={affiliation}
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
              icon={<Hash className="h-3.5 w-3.5 text-muted-foreground/70" />}
              onClick={() => void goChannel(row.channelId)}
              people={people}
              peopleTestId="project-channel-participants"
              profiles={profiles}
              testId="project-channel-row"
              title={`#${name}`}
              titleAttr={`Open #${name}`}
            />
          </li>
        );
      })}
    </ul>
  );
}
