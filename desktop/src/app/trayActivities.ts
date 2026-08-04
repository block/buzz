export type TrayAgentActivity = {
  activityId: string;
  agentName: string;
  channelId: string;
  channelName: string;
  elapsed: string;
};

/** Removes tray rows whose destination is absent from the active community. */
export function keepOpenableTrayActivities(
  activities: readonly TrayAgentActivity[],
  channelIds: ReadonlySet<string>,
): TrayAgentActivity[] {
  return activities.filter((activity) => channelIds.has(activity.channelId));
}
