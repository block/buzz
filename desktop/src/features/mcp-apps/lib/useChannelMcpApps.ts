import * as React from "react";

import {
  getChannelMcpApps,
  subscribeChannelMcpApps,
  type ChannelMcpAppInstallation,
} from "@/features/mcp-apps/lib/channelMcpAppStorage";

export function useChannelMcpApps({
  channelId,
  pubkey,
}: {
  channelId: string | null;
  pubkey: string | null | undefined;
}) {
  const getSnapshot = React.useCallback(() => {
    if (!channelId || !pubkey) return "[]";
    return JSON.stringify(getChannelMcpApps(pubkey, channelId));
  }, [channelId, pubkey]);
  const serialized = React.useSyncExternalStore(
    subscribeChannelMcpApps,
    getSnapshot,
    getSnapshot,
  );
  const apps = React.useMemo(
    () => JSON.parse(serialized) as ChannelMcpAppInstallation[],
    [serialized],
  );
  const [selection, setSelection] = React.useState<{
    channelId: string | null;
    appId: string | null;
  }>({ channelId, appId: null });
  const selectedAppId =
    selection.channelId === channelId ? selection.appId : null;
  const activeApp =
    apps.find((installation) => installation.id === selectedAppId) ?? null;
  const activeAppId = activeApp?.id ?? null;
  React.useEffect(() => {
    if (selectedAppId && !activeApp) {
      setSelection({ channelId, appId: null });
    }
  }, [activeApp, channelId, selectedAppId]);

  return {
    apps,
    activeApp,
    activeAppId,
    activateApp: React.useCallback(
      (appId: string) => setSelection({ channelId, appId }),
      [channelId],
    ),
    showChat: React.useCallback(
      () => setSelection({ channelId, appId: null }),
      [channelId],
    ),
  };
}
