import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  type ProjectRepoUnavailableReason,
  refineRepoUnavailableReason,
} from "@/features/projects/lib/projectRepoAvailability";
import { selectProjectRepository } from "@/features/projects/projectModels";

/** Channel ids the viewer is a member of, or `null` while they load. */
export function useMemberChannelIds(): readonly string[] | null {
  const channelsQuery = useChannelsQuery();
  return React.useMemo(
    () =>
      channelsQuery.data
        ? channelsQuery.data
            .filter((channel) => channel.isMember)
            .map((channel) => channel.id)
        : null,
    [channelsQuery.data],
  );
}

/**
 * Channel-ACL denials arrive as the same 404 as a missing repository
 * (anti-enumeration), so a project's raw snapshot reason is re-classified
 * with the repository's channel binding and the viewer's memberships
 * before it reaches the status indicator.
 */
export function useRepositoryUnavailableReasonFor(
  unavailable: Record<string, ProjectRepoUnavailableReason> | undefined,
  memberChannelIds: readonly string[] | null,
): (project: Project) => ProjectRepoUnavailableReason | undefined {
  return React.useCallback(
    (project: Project) => {
      const reason = unavailable?.[project.id];
      if (!reason) return undefined;
      return refineRepoUnavailableReason({
        reason,
        repositoryChannelId: selectProjectRepository(project, null)?.channelId,
        memberChannelIds,
      });
    },
    [memberChannelIds, unavailable],
  );
}
