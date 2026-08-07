import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";

export function ProjectOriginReference({
  agentName,
  channelId,
}: {
  agentName?: string | null;
  channelId?: string | null;
}) {
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery({ enabled: Boolean(channelId) });
  const channel = channelsQuery.data?.find(
    (candidate) => candidate.id === channelId,
  );

  if (channelId) {
    return (
      <span
        className="inline-flex min-w-0 items-center gap-1"
        title="Origin is claimed by the event author and is not relay-verified."
      >
        <span>started from</span>
        {channel ? (
          <button
            aria-label={`Open author-claimed origin channel #${channel.name}`}
            className="truncate font-medium text-foreground underline-offset-2 hover:underline"
            onClick={() => void goChannel(channel.id)}
            type="button"
          >
            #{channel.name}
          </button>
        ) : (
          <span>a public channel</span>
        )}
        <span>(author-claimed)</span>
      </span>
    );
  }

  if (agentName) {
    return (
      <span
        className="inline-flex min-w-0 items-center gap-1"
        title="The private conversation identifier is intentionally omitted."
      >
        <span>started privately with</span>
        <span className="truncate font-medium text-foreground">
          {agentName}
        </span>
      </span>
    );
  }

  return null;
}
