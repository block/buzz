import { createFileRoute } from "@tanstack/react-router";

import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { HomeScreen } from "@/features/home/ui/HomeScreen";

export const Route = createFileRoute("/")({
  component: HomeRouteComponent,
});

function HomeRouteComponent() {
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const channels = channelsQuery.data ?? [];
  const availableChannelIds = new Set(channels.map((channel) => channel.id));

  return (
    <HomeScreen
      availableChannelIds={availableChannelIds}
      currentPubkey={identityQuery.data?.pubkey}
    />
  );
}
