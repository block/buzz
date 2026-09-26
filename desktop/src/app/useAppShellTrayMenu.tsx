import type { Channel } from "@/shared/api/types";
import { isMacPlatform } from "@/shared/lib/platform";

import { useTrayMenu } from "@/app/useTrayMenu";

/** Keeps the ticking native tray menu outside AppShell's render cycle. */
export function AppShellTrayMenu({
  channels,
  goChannel,
  openCreateChannel,
  openSettings,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  openCreateChannel: () => void;
  openSettings: () => void;
}) {
  if (!isMacPlatform()) return null;
  return (
    <MacAppShellTrayMenu
      channels={channels}
      goChannel={goChannel}
      openCreateChannel={openCreateChannel}
      openSettings={openSettings}
    />
  );
}

function MacAppShellTrayMenu({
  channels,
  goChannel,
  openCreateChannel,
  openSettings,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  openCreateChannel: () => void;
  openSettings: () => void;
}): null {
  useTrayMenu({
    channels,
    goChannel,
    openCreateChannel,
    openSettings,
  });
  return null;
}
