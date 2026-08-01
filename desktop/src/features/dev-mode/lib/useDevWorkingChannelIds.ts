import * as React from "react";

import { useWorkingChannels } from "@/features/agents/agentWorkingSignal";
import {
  aggregateWorkingMains,
  type SubChannelIndex,
} from "@/features/dev-mode/lib/subChannels";

/** Exact working channels plus the parent ids that represent them in nav. */
export function useDevWorkingChannelIds(subIndex: SubChannelIndex) {
  const workingChannels = useWorkingChannels();
  const workingChannelIds = React.useMemo(
    () => new Set(workingChannels.map((summary) => summary.channelId)),
    [workingChannels],
  );
  const navigatorWorkingIds = React.useMemo(
    () => aggregateWorkingMains(subIndex, workingChannelIds),
    [subIndex, workingChannelIds],
  );
  return [workingChannelIds, navigatorWorkingIds] as const;
}
