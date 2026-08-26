import * as React from "react";

import { useChannelWebsitesQuery } from "@/features/channels/hooks";
import type { ChannelWebsite } from "@/features/channels/lib/channelWebsites";

export function useChannelWebsiteSurface(channelId: string | null): {
  websites: ChannelWebsite[];
  channelSurface: "chat" | string;
  setChannelSurface: (surface: "chat" | string) => void;
  activeWebsite: ChannelWebsite | null;
  websitesQuery: ReturnType<typeof useChannelWebsitesQuery>;
} {
  const [channelSurface, setChannelSurface] = React.useState<"chat" | string>(
    "chat",
  );
  const [surfaceChannelId, setSurfaceChannelId] = React.useState(channelId);
  if (channelId !== surfaceChannelId) {
    setSurfaceChannelId(channelId);
    setChannelSurface("chat");
  }

  const websitesQuery = useChannelWebsitesQuery(channelId, channelId !== null);
  const websites = websitesQuery.data ?? [];

  React.useEffect(() => {
    if (
      channelSurface !== "chat" &&
      !websites.some((site) => site.id === channelSurface)
    ) {
      setChannelSurface("chat");
    }
  }, [channelSurface, websites]);

  return {
    websites,
    channelSurface,
    setChannelSurface,
    activeWebsite:
      channelSurface === "chat"
        ? null
        : (websites.find((site) => site.id === channelSurface) ?? null),
    websitesQuery,
  };
}
